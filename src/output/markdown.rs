use std::collections::HashSet;
use std::fmt::Write;
use std::io::Write as IoWrite;

use arboard::Clipboard;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::app::{CommentTypeDefinition, DiffSource};
use crate::config::ExportConfig;
use crate::error::{Result, TuicrError};
use crate::forge::remote_comments::{
    PrCommentsVisibility, RemoteReviewThread, filter_threads, group_threads_by_path,
    remote_thread_overlay_for_session,
};
use crate::model::comment::DEFAULT_AUTHOR;
use crate::model::thread::ThreadStatus;
use crate::model::{CommentType, LineRange, LineSide, ReviewSession};
use crate::slug::short_sha;
/// (file_path, line_range, side, comment_type, content, commit_id, comment_id, author)
type CommentEntry<'a> = (
    String,
    Option<LineRange>,
    Option<LineSide>,
    String,
    &'a str,
    Option<&'a str>,
    &'a str,
    &'a str,
);

/// Generate markdown content from the review session.
/// Returns the markdown string or an error if there are no comments.
pub fn generate_export_content(
    session: &ReviewSession,
    diff_source: &DiffSource,
    comment_types: &[CommentTypeDefinition],
    export: &ExportConfig,
    remote_threads: &[RemoteReviewThread],
    session_slug: Option<&str>,
) -> Result<String> {
    // In PR mode it's still useful to export PR identity + remote
    // discussions even if the user has no local drafts. Outside PR mode
    // we keep the existing behavior of erroring when nothing is to say.
    let has_remote = matches!(diff_source, DiffSource::PullRequest(_))
        && !filter_threads(remote_threads, PrCommentsVisibility::Unresolved).is_empty();
    if !session.has_comments() && !has_remote {
        return Err(TuicrError::NoComments);
    }
    Ok(generate_markdown(
        session,
        diff_source,
        comment_types,
        export,
        remote_threads,
        session_slug,
    ))
}

pub fn export_to_clipboard(
    session: &ReviewSession,
    diff_source: &DiffSource,
    comment_types: &[CommentTypeDefinition],
    export: &ExportConfig,
    remote_threads: &[RemoteReviewThread],
    session_slug: Option<&str>,
) -> Result<String> {
    let content = generate_export_content(
        session,
        diff_source,
        comment_types,
        export,
        remote_threads,
        session_slug,
    )?;
    let via_terminal = copy_text_to_clipboard(&content)?;
    Ok(if via_terminal {
        "Review copied to clipboard (via terminal)".to_string()
    } else {
        "Review copied to clipboard".to_string()
    })
}

/// Copy arbitrary text to the system clipboard. Returns `Ok(true)` if the
/// terminal-based fallback (tmux/OSC 52) was used, `Ok(false)` if the
/// platform clipboard handled it.
pub fn copy_text_to_clipboard(text: &str) -> Result<bool> {
    // On macOS, pbcopy writes straight to the system pasteboard and works even
    // inside tmux/SSH. OSC 52 (preferred below) instead relies on the outer
    // terminal honoring the escape, which Terminal.app does not, so the copy
    // would only reach the tmux buffer. Prefer pbcopy unconditionally here.
    if cfg!(target_os = "macos") && try_clipboard_cmd("pbcopy", &[], text) {
        return Ok(false);
    }
    if should_prefer_osc52() {
        copy_osc52(text)?;
        return Ok(true);
    }
    if try_copy_via_subprocess(text) {
        return Ok(false);
    }
    match Clipboard::new().and_then(|mut cb| cb.set_text(text)) {
        Ok(_) => Ok(false),
        Err(_) => {
            copy_osc52(text)?;
            Ok(true)
        }
    }
}

/// Try xclip (X11) then wl-copy (Wayland). Returns true if either succeeds.
fn try_copy_via_subprocess(text: &str) -> bool {
    let session = std::env::var("XDG_SESSION_TYPE");
    if session.is_err() {
        // Session not specified
        return false;
    }
    let session = session.unwrap();
    if session == "wayland" {
        try_clipboard_cmd("wl-copy", &[], text)
    } else if session == "x11" {
        try_clipboard_cmd("xclip", &["-selection", "clipboard"], text)
    } else {
        // Session type unsupported
        false
    }
}

/// Try copying via a CLI tool that forks into the background and holds
/// clipboard ownership beyond tuicr's process lifetime. Returns true on success.
fn try_clipboard_cmd(program: &str, args: &[&str], text: &str) -> bool {
    use std::process::{Command, Stdio};
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = std::io::Write::write_all(&mut stdin, text.as_bytes());
    }
    matches!(child.wait(), Ok(s) if s.success())
}

/// Returns true if we should prefer OSC 52 over the system clipboard.
///
/// In tmux or SSH sessions, arboard may "succeed" but copy to an inaccessible
/// X11 clipboard, so we use OSC 52 which works reliably in these environments.
fn should_prefer_osc52() -> bool {
    std::env::var("TMUX").is_ok()
        || std::env::var("SSH_TTY").is_ok()
        || std::env::var("ZELLIJ").is_ok()
}

/// Copy text to clipboard using OSC 52 escape sequence.
/// In tmux, raw OSC 52 is intercepted and may not reach the outer terminal.
/// We use `tmux load-buffer -w` which tells tmux to handle the clipboard copy itself.
fn copy_osc52(text: &str) -> Result<()> {
    if std::env::var("TMUX").is_ok() {
        copy_via_tmux(text)
    } else {
        let mut stdout = std::io::stdout().lock();
        write_osc52(&mut stdout, text)
    }
}

/// Copy text to the system clipboard via `tmux load-buffer -w -`.
/// The `-w` flag tells tmux to also forward to the outer terminal's clipboard via OSC 52.
fn copy_via_tmux(text: &str) -> Result<()> {
    use std::process::{Command, Stdio};

    let mut child = Command::new("tmux")
        .args(["load-buffer", "-w", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| TuicrError::Clipboard(format!("Failed to run tmux: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| TuicrError::Clipboard(format!("Failed to write to tmux: {e}")))?;
    }

    let status = child
        .wait()
        .map_err(|e| TuicrError::Clipboard(format!("tmux load-buffer failed: {e}")))?;

    if !status.success() {
        return Err(TuicrError::Clipboard(
            "tmux load-buffer exited with error".to_string(),
        ));
    }

    Ok(())
}

/// Write OSC 52 escape sequence to the given writer.
/// Separated for testability.
fn write_osc52<W: IoWrite>(writer: &mut W, text: &str) -> Result<()> {
    let encoded = BASE64.encode(text);
    write!(writer, "\x1b]52;c;{encoded}\x07")
        .map_err(|e| TuicrError::Clipboard(format!("Failed to write OSC 52: {e}")))?;
    writer
        .flush()
        .map_err(|e| TuicrError::Clipboard(format!("Failed to flush: {e}")))?;
    Ok(())
}

fn review_scope_label(diff_source: &DiffSource) -> String {
    let scope = match diff_source {
        DiffSource::WorkingTree => "working tree changes".to_string(),
        DiffSource::StagedAndUnstaged => "staged + unstaged changes".to_string(),
        DiffSource::Staged => "staged changes".to_string(),
        DiffSource::Unstaged => "unstaged changes".to_string(),
        DiffSource::CommitRange(_) => "selected commit range".to_string(),
        DiffSource::StagedUnstagedAndCommits(_) => {
            "selected commit range + staged/unstaged changes".to_string()
        }
        DiffSource::PullRequest(pr) => format!(
            "pull request {}#{}",
            pr.key.repository.display_name(),
            pr.key.number
        ),
    };

    format!("Review Comment (scope: {scope})")
}

/// The "Reviewing …" banner for non-pull-request scopes. `None` for the
/// working tree, which has never carried one, and for pull requests, whose
/// banner is emitted alongside their metadata.
fn scope_banner(diff_source: &DiffSource) -> Option<String> {
    fn short_ids(commits: &[String]) -> String {
        commits
            .iter()
            .map(|c| &c[..7.min(c.len())])
            .collect::<Vec<_>>()
            .join(", ")
    }

    match diff_source {
        DiffSource::WorkingTree | DiffSource::PullRequest(_) => None,
        DiffSource::Staged => Some("Reviewing staged changes".to_string()),
        DiffSource::Unstaged => Some("Reviewing unstaged changes".to_string()),
        DiffSource::StagedAndUnstaged => Some("Reviewing staged + unstaged changes".to_string()),
        DiffSource::CommitRange(commits) if commits.len() == 1 => Some(format!(
            "Reviewing commit: {}",
            &commits[0][..7.min(commits[0].len())]
        )),
        DiffSource::CommitRange(commits) => {
            Some(format!("Reviewing commits: {}", short_ids(commits)))
        }
        DiffSource::StagedUnstagedAndCommits(commits) => Some(format!(
            "Reviewing staged + unstaged + commits: {}",
            short_ids(commits)
        )),
    }
}

fn generate_markdown(
    session: &ReviewSession,
    diff_source: &DiffSource,
    comment_types: &[CommentTypeDefinition],
    export: &ExportConfig,
    remote_threads: &[RemoteReviewThread],
    session_slug: Option<&str>,
) -> String {
    let mut md = String::new();

    if let Some(slug) = session_slug {
        let _ = writeln!(md, "## Session: {slug}");
        let _ = writeln!(md);
    }

    // Intro for agents. An empty string drops the line and its spacer so the
    // export opens directly on content.
    let intro = export.intro();
    if !intro.is_empty() {
        let _ = writeln!(md, "{intro}");
        let _ = writeln!(md);
    }

    // Scope banner. `scope_line` governs the prose "what am I reviewing" line;
    // `pr_metadata` separately governs the PR URL and head SHA, which an agent
    // needs to fetch context even when the preamble has been trimmed away.
    match diff_source {
        DiffSource::PullRequest(pr) => {
            let mut wrote_any = false;
            if export.scope_line() {
                let _ = writeln!(
                    md,
                    "Reviewing pull request {}#{}: {}",
                    pr.key.repository.display_name(),
                    pr.key.number,
                    pr.title
                );
                wrote_any = true;
            }
            if export.pr_metadata() {
                let _ = writeln!(md, "URL: {}", pr.url);
                let _ = writeln!(md, "Head: {}", pr.key.short_head());
                wrote_any = true;
            }
            if wrote_any {
                let _ = writeln!(md);
            }
        }
        other => {
            if export.scope_line()
                && let Some(banner) = scope_banner(other)
            {
                let _ = writeln!(md, "{banner}");
                let _ = writeln!(md);
            }
        }
    }

    if export.legend() {
        let used_ids = collect_used_comment_type_ids(session);
        // The typeless `None` default never appears in the legend.
        let legend = comment_types
            .iter()
            .filter(|ct| ct.id != CommentType::NONE_ID)
            .filter(|ct| used_ids.is_empty() || used_ids.contains(&ct.id))
            .map(|comment_type| {
                let definition = comment_type
                    .definition
                    .as_deref()
                    .unwrap_or(comment_type.id.as_str());
                format!(
                    "{} ({})",
                    comment_type.label.to_ascii_uppercase(),
                    definition
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        // Omit the line entirely when there are no typed comments to document
        // (e.g. an untyped-only review).
        if !legend.is_empty() {
            let _ = writeln!(md, "Comment types: {legend}");
            let _ = writeln!(md);
        }
    }

    // Session notes/summary
    if let Some(notes) = &session.session_notes {
        let _ = writeln!(md, "Summary: {notes}");
        let _ = writeln!(md);
    }

    // Collect all comments into a flat list
    let mut all_comments: Vec<CommentEntry> = Vec::new();
    let review_comment_location = review_scope_label(diff_source);

    for comment in &session.review_comments {
        all_comments.push((
            review_comment_location.clone(),
            None,
            None,
            export_comment_type_label(&comment.comment_type, comment_types),
            &comment.content,
            None,
            comment.id.as_str(),
            comment.author.as_str(),
        ));
    }

    // Sort files by path for consistent output
    let mut files: Vec<_> = session.files.iter().collect();
    files.sort_by_key(|(path, _)| path.to_string_lossy().to_string());

    for (path, review) in files {
        let path_str = path.display().to_string();

        // File comments (no line number)
        for comment in &review.file_comments {
            all_comments.push((
                path_str.clone(),
                None,
                None,
                export_comment_type_label(&comment.comment_type, comment_types),
                &comment.content,
                comment.commit_id.as_deref(),
                comment.id.as_str(),
                comment.author.as_str(),
            ));
        }

        // Line comments (with line number, sorted)
        let mut line_comments: Vec<_> = review.line_comments.iter().collect();
        line_comments.sort_by_key(|(line, _)| *line);

        for (line, comments) in line_comments {
            for comment in comments {
                // Use comment's line_range if available, otherwise use the key line
                let line_range = comment
                    .line_range
                    .or_else(|| Some(LineRange::single(*line)));
                all_comments.push((
                    path_str.clone(),
                    line_range,
                    comment.side,
                    export_comment_type_label(&comment.comment_type, comment_types),
                    &comment.content,
                    comment.commit_id.as_deref(),
                    comment.id.as_str(),
                    comment.author.as_str(),
                ));
            }
        }
    }

    // Output numbered list. An empty header drops the heading but still marks
    // the section as written, so the remote-section separator below survives.
    let mut local_section_written = false;
    if !all_comments.is_empty() {
        let header = export.comments_header();
        if !header.is_empty() {
            let _ = writeln!(md, "{header}");
            let _ = writeln!(md);
        }
        local_section_written = true;
    }
    // A durable Thread can mirror several legacy comments grouped at one
    // anchor (see `ReviewSession::migrate_legacy_comments_to_threads`), so
    // native-only replies (no legacy `Comment` counterpart — see
    // `App::reply_to_thread_at_cursor`) must print exactly once, after the
    // *last* legacy comment in the group — see
    // `ReviewSession::is_last_legacy_comment_for_thread`, the single shared
    // predicate this loop, `App::splice_native_thread_replies`, and
    // `ui::diff_view::push_native_thread_replies` all use, so the three
    // can never independently drift into different orderings.
    for (i, (file, line_range, side, comment_type, content, commit_id, comment_id, author)) in
        all_comments.iter().enumerate()
    {
        let number = i + 1;
        let location = match (line_range, side) {
            // Range on deleted side (old lines)
            (Some(range), Some(LineSide::Old)) if range.is_single() => {
                format!("`{}:~{}`", file, range.start)
            }
            (Some(range), Some(LineSide::Old)) => {
                format!("`{}:~{}-~{}`", file, range.start, range.end)
            }
            // Range on new/context side
            (Some(range), _) if range.is_single() => {
                format!("`{}:{}`", file, range.start)
            }
            (Some(range), _) => {
                format!("`{}:{}-{}`", file, range.start, range.end)
            }
            // File comment
            (None, _) => format!("`{file}`"),
        };
        // Append the commit short SHA so the LLM knows which commit the
        // comment was made against — crucial for per-commit reviews.
        let commit_suffix = match commit_id {
            Some(sha) => format!(" (commit {})", short_sha(sha)),
            None => String::new(),
        };
        // Thread canonical state: surface authorship (mirroring the TUI's
        // `format_comment_lines`, which only calls out an author that isn't
        // the acting user) and any non-`Open` thread status (stale /
        // ambiguous / resolved / dismissed) so an agent reading the export
        // knows a comment's discussion has moved on without needing the
        // separate `tuicr review thread` JSON API.
        let thread = session.find_thread_by_legacy_comment_id(comment_id);
        let author_suffix = if *author == DEFAULT_AUTHOR {
            String::new()
        } else {
            format!(" @{author}")
        };
        let status_suffix = thread
            .map(|persisted| persisted.thread.status())
            .filter(|status| !matches!(status, ThreadStatus::Open))
            .map(|status| format!(" ({})", thread_status_label(status)))
            .unwrap_or_default();
        let marker = format!("{number}.");
        let continuation_indent = " ".repeat(marker.len() + 1);
        let mut content_lines = content.split('\n').map(|line| line.trim_end_matches('\r'));
        let first_line = content_lines.next().unwrap_or_default();
        // Untyped (`None`) comments export with no `**[TYPE]**` marker.
        let type_marker = if comment_type.is_empty() {
            String::new()
        } else {
            format!("**[{comment_type}]** ")
        };
        let _ = writeln!(
            md,
            "{marker} {type_marker}{location}{commit_suffix}{author_suffix}{status_suffix} - {first_line}"
        );
        for line in content_lines {
            let _ = writeln!(md, "{continuation_indent}{line}");
        }

        // Thread-native replies with no legacy `Comment` counterpart (added
        // via the TUI's thread-reply keybinding) have no other export
        // representation at all, so surface them indented under the root
        // comment they belong to, once per thread, after the last
        // grouped legacy comment.
        if let Some(persisted) = thread
            && session.is_last_legacy_comment_for_thread(comment_id)
        {
            for reply in persisted.thread.comments() {
                if session.is_legacy_comment_id(reply.id().as_str()) {
                    continue;
                }
                let mut reply_lines = reply.body.split('\n');
                let reply_first = reply_lines.next().unwrap_or_default();
                let _ = writeln!(
                    md,
                    "{continuation_indent}\u{21b3} @{} - {reply_first}",
                    reply.author.name
                );
                for line in reply_lines {
                    let _ = writeln!(md, "{continuation_indent}  {line}");
                }
            }
        }
    }

    // PR-mode-only: include unresolved remote discussions grouped by file.
    if matches!(diff_source, DiffSource::PullRequest(_)) {
        let unresolved: Vec<&RemoteReviewThread> =
            filter_threads(remote_threads, PrCommentsVisibility::Unresolved);
        if !unresolved.is_empty() {
            if local_section_written {
                let _ = writeln!(md);
            }
            let remote_header = export.remote_comments_header();
            if !remote_header.is_empty() {
                let _ = writeln!(md, "{remote_header}");
                let _ = writeln!(md);
            }

            let provider = match diff_source {
                DiffSource::PullRequest(pr) => Some(pr.key.repository.kind.provider_key()),
                _ => None,
            };

            // Group threads by file to make the export easy to scan.
            let owned_unresolved: Vec<RemoteReviewThread> =
                unresolved.iter().map(|t| (*t).clone()).collect();
            let groups = group_threads_by_path(&owned_unresolved);
            let mut thread_n = 1;
            for (path, threads) in groups {
                let _ = writeln!(md, "### `{path}`");
                let _ = writeln!(md);
                for thread in threads {
                    if let Some(root) = thread.root() {
                        // Durable-local overlay: a reply/resolve/dismiss made
                        // in the TUI against the thread this remote DTO was
                        // imported into (see `remote_thread_overlay_for_session`
                        // doc comment) — the DTO itself is never mutated, so
                        // without this the export would silently drop any
                        // local activity on a remote-imported thread.
                        let overlay = remote_thread_overlay_for_session(session, provider, thread);
                        let author = root.author.as_deref().unwrap_or("unknown");
                        let line_marker = thread.line.map(|l| format!(":{l}")).unwrap_or_default();
                        // Local status only adds a suffix when it carries
                        // information the remote `is_resolved` flag doesn't
                        // already convey (mirrors
                        // `ui::comment_panel::local_status_badge_suffix`).
                        let local_status_suffix = overlay
                            .as_ref()
                            .map(|o| o.local_status)
                            .filter(|status| {
                                !matches!(status, ThreadStatus::Open)
                                    && !(matches!(status, ThreadStatus::Resolved)
                                        && thread.is_resolved)
                            })
                            .map(|status| format!(" (locally {})", thread_status_label(status)))
                            .unwrap_or_default();
                        let _ = writeln!(
                            md,
                            "{thread_n}. `{path}{line_marker}` @{author}{local_status_suffix} - {body}",
                            body = root.body
                        );
                        if !root.url.is_empty() {
                            let _ = writeln!(md, "   <{}>", root.url);
                        }
                        for reply in thread.replies() {
                            let reply_author = reply.author.as_deref().unwrap_or("unknown");
                            let _ =
                                writeln!(md, "   - @{reply_author} - {body}", body = reply.body);
                        }
                        // Local-only replies (not already represented by a
                        // provider/comment ID on the remote DTO) — appended
                        // strictly after the remote-authored replies,
                        // preserving order, same convention as
                        // `merge_remote_thread_into_existing`.
                        if let Some(overlay) = &overlay {
                            for reply in &overlay.local_only_replies {
                                let _ = writeln!(
                                    md,
                                    "   - @{} (local) - {body}",
                                    reply.author.name,
                                    body = reply.body
                                );
                            }
                        }
                        thread_n += 1;
                    }
                }
                let _ = writeln!(md);
            }
        }
    }

    md
}

/// Lowercase, human-readable label for a non-`Open` [`ThreadStatus`], mirroring
/// the suffix convention already used by `ui::comment_panel::thread_status_suffix`
/// for TUI rendering. Callers only invoke this after filtering out `Open`.
fn thread_status_label(status: ThreadStatus) -> &'static str {
    match status {
        ThreadStatus::Open => "open",
        ThreadStatus::Stale => "stale",
        ThreadStatus::Ambiguous => "ambiguous",
        ThreadStatus::Resolved => "resolved",
        ThreadStatus::Dismissed => "dismissed",
    }
}

fn collect_used_comment_type_ids(session: &ReviewSession) -> HashSet<String> {
    let mut ids = HashSet::new();
    for c in &session.review_comments {
        ids.insert(c.comment_type.id().to_string());
    }
    for review in session.files.values() {
        for c in &review.file_comments {
            ids.insert(c.comment_type.id().to_string());
        }
        for comments in review.line_comments.values() {
            for c in comments {
                ids.insert(c.comment_type.id().to_string());
            }
        }
    }
    ids
}

/// Export label for a comment type. Returns an empty string for
/// [`CommentType::None`] so the caller omits the `**[TYPE]**` marker.
fn export_comment_type_label(
    comment_type: &CommentType,
    comment_types: &[CommentTypeDefinition],
) -> String {
    if comment_type.is_none() {
        return String::new();
    }

    if let Some(definition) = comment_types
        .iter()
        .find(|definition| definition.id == comment_type.id())
    {
        return definition.label.to_ascii_uppercase();
    }

    comment_type.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::CommentTypeDefinition;
    use crate::model::{Comment, CommentType, FileStatus, LineRange, LineSide, SessionDiffSource};
    use std::path::PathBuf;

    /// Export settings with only the legend suppressed.
    fn legend_off() -> ExportConfig {
        ExportConfig {
            legend: Some(false),
            ..Default::default()
        }
    }

    /// A PR session carrying one local draft comment, for exercising the
    /// boundary between the local and remote sections.
    fn pr_session_with_local_draft() -> ReviewSession {
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abc1234deadbeef".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        session.add_file(PathBuf::from("src/lib.rs"), FileStatus::Modified, 0);
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/lib.rs")) {
            review.add_line_comment(
                10,
                Comment::new(
                    "Local draft".to_string(),
                    CommentType::from_id("issue"),
                    Some(LineSide::New),
                ),
            );
        }
        session
    }

    #[test]
    fn should_use_custom_comment_headers() {
        let session = pr_session_with_local_draft();
        let export = ExportConfig {
            comments_header: Some("## Comments".to_string()),
            remote_comments_header: Some("## Upstream".to_string()),
            ..Default::default()
        };

        let markdown = generate_markdown(
            &session,
            &sample_pr_diff_source(),
            &comment_types(),
            &export,
            &[sample_remote_thread(
                "a",
                "alice",
                "Can this be simpler?",
                42,
                false,
            )],
            None,
        );

        assert!(markdown.contains("## Comments"));
        assert!(markdown.contains("## Upstream"));
        assert!(!markdown.contains("## Local tuicr Comments"));
        assert!(!markdown.contains("## Existing GitHub Comments"));
    }

    #[test]
    fn should_keep_remote_separator_when_comments_header_is_empty() {
        // Dropping the local heading must not swallow the blank line that
        // separates the numbered list from the remote section.
        let session = pr_session_with_local_draft();
        let export = ExportConfig {
            comments_header: Some(String::new()),
            ..Default::default()
        };

        let markdown = generate_markdown(
            &session,
            &sample_pr_diff_source(),
            &comment_types(),
            &export,
            &[sample_remote_thread(
                "a",
                "alice",
                "Can this be simpler?",
                42,
                false,
            )],
            None,
        );

        assert!(!markdown.contains("## Local tuicr Comments"));
        assert!(
            markdown.contains("\n\n## Existing GitHub Comments"),
            "expected a blank line before the remote header in:\n{markdown}"
        );
    }

    #[test]
    fn should_omit_scope_line_when_disabled() {
        let session = create_test_session();
        let export = ExportConfig {
            scope_line: Some(false),
            ..Default::default()
        };

        let markdown = generate_markdown(
            &session,
            &DiffSource::Unstaged,
            &comment_types(),
            &export,
            &[],
            None,
        );

        assert!(!markdown.contains("Reviewing unstaged changes"));
    }

    #[test]
    fn should_keep_pr_metadata_when_scope_line_disabled() {
        // URL and head SHA are addressable context an agent needs, not
        // preamble framing, so trimming the scope line must not drop them.
        let session = create_test_session();
        let export = ExportConfig {
            scope_line: Some(false),
            ..Default::default()
        };

        let markdown = generate_markdown(
            &session,
            &sample_pr_diff_source(),
            &comment_types(),
            &export,
            &[],
            None,
        );

        assert!(!markdown.contains("Reviewing pull request"));
        assert!(markdown.contains("URL: https://github.com/agavra/tuicr/pull/125"));
        assert!(markdown.contains("Head: "));
    }

    #[test]
    fn should_omit_pr_metadata_when_disabled_but_keep_scope_line() {
        let session = create_test_session();
        let export = ExportConfig {
            pr_metadata: Some(false),
            ..Default::default()
        };

        let markdown = generate_markdown(
            &session,
            &sample_pr_diff_source(),
            &comment_types(),
            &export,
            &[],
            None,
        );

        assert!(markdown.contains("Reviewing pull request agavra/tuicr#125"));
        assert!(!markdown.contains("URL: "));
        assert!(!markdown.contains("Head: "));
    }

    #[test]
    fn should_omit_pr_banner_and_spacer_when_scope_and_metadata_disabled() {
        let session = create_test_session();
        let export = ExportConfig {
            intro: Some(String::new()),
            scope_line: Some(false),
            pr_metadata: Some(false),
            ..Default::default()
        };

        let markdown = generate_markdown(
            &session,
            &sample_pr_diff_source(),
            &comment_types(),
            &export,
            &[],
            None,
        );

        assert!(!markdown.contains("Reviewing pull request"));
        assert!(!markdown.contains("URL: "));
        assert!(
            !markdown.starts_with('\n'),
            "the banner's trailing blank line should go with it, got:\n{markdown}"
        );
    }

    #[test]
    fn should_use_custom_export_intro() {
        let session = create_test_session();
        let export = ExportConfig {
            intro: Some("Code review comments:".to_string()),
            ..Default::default()
        };

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &export,
            &[],
            None,
        );

        assert!(markdown.contains("Code review comments:"));
        assert!(!markdown.contains("I reviewed your code"));
    }

    #[test]
    fn should_omit_export_intro_and_its_spacer_when_empty() {
        let session = create_test_session();
        let export = ExportConfig {
            intro: Some(String::new()),
            ..Default::default()
        };

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &export,
            &[],
            None,
        );

        assert!(!markdown.contains("I reviewed your code"));
        assert!(
            !markdown.starts_with('\n'),
            "the intro's trailing blank line should go with it, got:\n{markdown}"
        );
    }

    fn comment_types() -> Vec<CommentTypeDefinition> {
        vec![
            CommentTypeDefinition {
                id: "note".to_string(),
                label: "note".to_string(),
                definition: Some("observations".to_string()),
                color: None,
            },
            CommentTypeDefinition {
                id: "suggestion".to_string(),
                label: "suggestion".to_string(),
                definition: Some("improvements".to_string()),
                color: None,
            },
            CommentTypeDefinition {
                id: "issue".to_string(),
                label: "issue".to_string(),
                definition: Some("problems to fix".to_string()),
                color: None,
            },
            CommentTypeDefinition {
                id: "praise".to_string(),
                label: "praise".to_string(),
                definition: Some("positive feedback".to_string()),
                color: None,
            },
        ]
    }

    fn create_test_session() -> ReviewSession {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);

        // Add a file comment
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.reviewed = true;
            review.add_file_comment(Comment::new(
                "Consider adding documentation".to_string(),
                CommentType::from_id("suggestion"),
                None,
            ));
            review.add_line_comment(
                42,
                Comment::new(
                    "Magic number should be a constant".to_string(),
                    CommentType::from_id("issue"),
                    Some(LineSide::New),
                ),
            );
        }

        session
    }

    #[test]
    fn should_include_session_slug_header_when_provided() {
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;

        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            Some("agavra/tuicr@main/worktree"),
        );

        assert!(
            markdown.contains("## Session: agavra/tuicr@main/worktree"),
            "expected slug header in:\n{markdown}"
        );
    }

    #[test]
    fn should_omit_session_slug_header_when_absent() {
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;

        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        assert!(!markdown.contains("## Session:"));
    }

    #[test]
    fn should_generate_valid_markdown() {
        // given
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        // then
        assert!(markdown.contains("I reviewed your code and have the following comments"));
        assert!(
            markdown.contains("Comment types: SUGGESTION (improvements), ISSUE (problems to fix)")
        );
        assert!(!markdown.contains("NOTE"));
        assert!(!markdown.contains("PRAISE"));
        assert!(markdown.contains("[SUGGESTION]"));
        assert!(markdown.contains("`src/main.rs`"));
        assert!(markdown.contains("Consider adding documentation"));
        assert!(markdown.contains("[ISSUE]"));
        assert!(markdown.contains("`src/main.rs:42`"));
        assert!(markdown.contains("Magic number"));
    }

    #[test]
    fn should_keep_commit_identity_for_commit_message_comments() {
        // Comments on the messages of different commits must not collapse into
        // an indistinguishable `Commit Message:N`. The synthetic commit-message
        // path carries the short id, which keeps each comment attributable to
        // its commit in the export.
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::CommitRange,
        );
        let first = PathBuf::from("Commit Message (ed50028)");
        let second = PathBuf::from("Commit Message (c17beb2)");
        session.add_file(first.clone(), FileStatus::Added, 0);
        session.add_file(second.clone(), FileStatus::Added, 0);
        if let Some(review) = session.get_file_mut(&first) {
            review.add_line_comment(
                1,
                Comment::new(
                    "We do not need this commit".to_string(),
                    CommentType::from_id("note"),
                    Some(LineSide::New),
                ),
            );
        }
        if let Some(review) = session.get_file_mut(&second) {
            review.add_line_comment(
                6,
                Comment::new(
                    "This is wrong".to_string(),
                    CommentType::from_id("note"),
                    Some(LineSide::New),
                ),
            );
        }

        let diff_source =
            DiffSource::CommitRange(vec!["ed50028".to_string(), "c17beb2".to_string()]);
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        assert!(
            markdown.contains("`Commit Message (ed50028):1`"),
            "expected first commit's message comment to be attributable in:\n{markdown}"
        );
        assert!(
            markdown.contains("`Commit Message (c17beb2):6`"),
            "expected second commit's message comment to be attributable in:\n{markdown}"
        );
    }

    #[test]
    fn should_use_configured_label_and_definition_in_export() {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.add_file_comment(Comment::new(
                "Needs clarification".to_string(),
                CommentType::from_id("note"),
                None,
            ));
        }

        let custom_types = vec![CommentTypeDefinition {
            id: "note".to_string(),
            label: "question".to_string(),
            definition: Some("ask for clarification".to_string()),
            color: None,
        }];

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &custom_types,
            &ExportConfig::default(),
            &[],
            None,
        );

        assert!(markdown.contains("Comment types: QUESTION (ask for clarification)"));
        assert!(markdown.contains("**[QUESTION]**"));
    }

    #[test]
    fn should_number_comments_sequentially() {
        // given
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        // then
        // Should have 2 numbered comments
        assert!(markdown.contains("1. **[SUGGESTION]**"));
        assert!(markdown.contains("2. **[ISSUE]**"));
    }

    #[test]
    fn should_show_author_suffix_when_comment_author_is_not_default() {
        // given a comment authored by someone other than the sentinel
        // default (`Comment::DEFAULT_AUTHOR` = "user")
        let mut session = create_test_session();
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.file_comments[0].author = "alice".to_string();
        }

        // when
        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        // then the non-default author is called out
        assert!(
            markdown.contains("@alice - Consider adding documentation"),
            "expected author callout in:\n{markdown}"
        );
        // and a default-author ("user") comment stays exactly as before
        // (no `@user` noise for the common single-reviewer case)
        assert!(
            markdown.contains("`src/main.rs:42` - Magic number should be a constant"),
            "expected no author suffix for default-author comment in:\n{markdown}"
        );
    }

    #[test]
    fn should_show_thread_status_suffix_for_migrated_non_open_thread() {
        // given a session whose file comment has been migrated to a durable
        // Thread (see `ReviewSession::migrate_legacy_comments_to_threads`)
        // and then resolved
        let mut session = create_test_session();
        session.migrate_legacy_comments_to_threads();
        let comment_id = session.files[&PathBuf::from("src/main.rs")].file_comments[0]
            .id
            .clone();
        let thread_id = session
            .find_thread_by_legacy_comment_id(&comment_id)
            .unwrap()
            .id()
            .clone();
        session
            .find_thread_mut(&thread_id)
            .unwrap()
            .thread
            .resolve();

        // when
        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        // then the resolved status is surfaced on the legacy comment's
        // export line, so an agent knows the discussion has moved on
        // without needing the separate `tuicr review thread` JSON API.
        assert!(
            markdown.contains("(resolved) - Consider adding documentation"),
            "expected resolved-status suffix in:\n{markdown}"
        );
    }

    #[test]
    fn should_render_native_thread_reply_with_no_legacy_comment_counterpart() {
        // given a migrated thread with a reply added directly (the same
        // mechanism `App::reply_to_thread_at_cursor` uses), which has no
        // legacy `Comment` counterpart at all
        let mut session = create_test_session();
        session.migrate_legacy_comments_to_threads();
        let comment_id = session.files[&PathBuf::from("src/main.rs")].file_comments[0]
            .id
            .clone();
        let thread_id = session
            .find_thread_by_legacy_comment_id(&comment_id)
            .unwrap()
            .id()
            .clone();
        session.find_thread_mut(&thread_id).unwrap().thread.reply(
            crate::model::thread::ThreadComment::new(
                crate::model::thread::ThreadAuthor::human("bob"),
                "Sounds good, thanks",
            ),
        );

        // when
        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        // then the native-only reply is visible, indented under its root
        assert!(
            markdown.contains("\u{21b3} @bob - Sounds good, thanks"),
            "expected native-only reply in:\n{markdown}"
        );
    }

    #[test]
    fn should_render_native_reply_once_per_group_not_once_per_grouped_legacy_comment() {
        // given two legacy line comments at the same anchor (migrated into
        // one thread — root + one legacy-derived reply) plus a native-only
        // reply on top
        let mut session = create_test_session();
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.add_line_comment(
                42,
                Comment::new(
                    "agreed, let's fix".to_string(),
                    CommentType::from_id("issue"),
                    Some(LineSide::New),
                ),
            );
        }
        session.migrate_legacy_comments_to_threads();
        let comments_at_42 = &session.files[&PathBuf::from("src/main.rs")].line_comments[&42];
        assert_eq!(
            comments_at_42.len(),
            2,
            "both legacy comments share one anchor"
        );
        let root_comment_id = comments_at_42[0].id.clone();
        let thread_id = session
            .find_thread_by_legacy_comment_id(&root_comment_id)
            .unwrap()
            .id()
            .clone();
        session.find_thread_mut(&thread_id).unwrap().thread.reply(
            crate::model::thread::ThreadComment::new(
                crate::model::thread::ThreadAuthor::human("carol"),
                "one native reply",
            ),
        );

        // when
        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        // then the native reply prints exactly once, not once per legacy
        // comment sharing the thread's anchor.
        assert_eq!(
            markdown
                .matches("\u{21b3} @carol - one native reply")
                .count(),
            1,
            "expected the native reply exactly once in:\n{markdown}"
        );

        // and it prints in the correct root -> legacy reply -> native
        // reply order (not merely present/counted once) - i.e. after
        // BOTH grouped legacy comment bodies, not spliced between them.
        let root_pos = markdown
            .find("Magic number should be a constant")
            .expect("root comment body rendered");
        let legacy_reply_pos = markdown
            .find("agreed, let's fix")
            .expect("legacy reply body rendered");
        let native_pos = markdown
            .find("one native reply")
            .expect("native reply body rendered");
        assert!(
            root_pos < legacy_reply_pos && legacy_reply_pos < native_pos,
            "expected markdown order root({root_pos}) < legacy reply({legacy_reply_pos}) < native({native_pos}) in:\n{markdown}"
        );
    }

    #[test]
    fn should_indent_multiline_comments_under_single_digit_list_marker() {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.add_file_comment(Comment::new(
                "foo\nbar".to_string(),
                CommentType::from_id("suggestion"),
                None,
            ));
        }

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        let expected = "\
1. **[SUGGESTION]** `src/main.rs` - foo
   bar";

        assert!(
            markdown.contains(expected),
            "expected multiline comment continuation to align under list text:\n{markdown}"
        );
    }

    #[test]
    fn should_indent_multiline_comments_under_double_digit_list_marker() {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            for i in 0..9 {
                review.add_file_comment(Comment::new(
                    format!("comment {i}"),
                    CommentType::from_id("note"),
                    None,
                ));
            }
            review.add_file_comment(Comment::new(
                "foo\nbar".to_string(),
                CommentType::from_id("suggestion"),
                None,
            ));
        }

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        let expected = "\
10. **[SUGGESTION]** `src/main.rs` - foo
    bar";

        assert!(
            markdown.contains(expected),
            "expected double-digit continuation to align under list text:\n{markdown}"
        );
    }

    #[test]
    fn should_include_review_comments_in_export() {
        let mut session = create_test_session();
        session.review_comments.push(Comment::new(
            "Please split this into smaller commits".to_string(),
            CommentType::from_id("note"),
            None,
        ));

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        assert!(markdown
            .contains("`Review Comment (scope: working tree changes)` - Please split this into smaller commits"));
    }

    #[test]
    fn should_include_commit_range_scope_for_review_comments() {
        let mut session = create_test_session();
        session.review_comments.push(Comment::new(
            "High-level concern across commits".to_string(),
            CommentType::from_id("issue"),
            None,
        ));

        let markdown = generate_markdown(
            &session,
            &DiffSource::CommitRange(vec!["abc1234567890".to_string()]),
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        assert!(markdown.contains(
            "`Review Comment (scope: selected commit range)` - High-level concern across commits"
        ));
    }

    #[test]
    fn should_include_commit_sha_in_export_for_commit_scoped_comments() {
        let mut session = create_test_session();
        // Add a line comment scoped to commit abc1234567890
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.add_line_comment(
                10,
                Comment::new(
                    "Wrong variable name".to_string(),
                    CommentType::from_id("issue"),
                    Some(LineSide::New),
                )
                .with_commit_id("abc1234567890"),
            );
        }

        let markdown = generate_markdown(
            &session,
            &DiffSource::CommitRange(vec!["abc1234567890".to_string()]),
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        assert!(
            markdown.contains("`src/main.rs:10` (commit abc1234) - Wrong variable name"),
            "commit-scoped comment must include the short SHA in the export; got:\n{markdown}"
        );
    }

    #[test]
    fn should_omit_commit_suffix_for_unscoped_comments() {
        let session = create_test_session();

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        // Existing comments have commit_id = None — no (commit ...) suffix
        assert!(
            !markdown.contains("(commit "),
            "unscoped comments must not have a commit suffix; got:\n{markdown}"
        );
    }

    #[test]
    fn should_fail_export_when_no_comments() {
        // given
        let session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        let diff_source = DiffSource::WorkingTree;

        // when
        let result = export_to_clipboard(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        // then
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), TuicrError::NoComments));
    }

    #[test]
    fn should_generate_export_content_with_comments() {
        // given
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;

        // when
        let result = generate_export_content(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        // then
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.contains("I reviewed your code"));
        assert!(content.contains("[SUGGESTION]"));
        assert!(content.contains("[ISSUE]"));
    }

    #[test]
    fn should_fail_generate_export_content_when_no_comments() {
        // given
        let session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        let diff_source = DiffSource::WorkingTree;

        // when
        let result = generate_export_content(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        // then
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), TuicrError::NoComments));
    }

    #[test]
    fn should_include_commit_range_in_markdown() {
        // given
        let session = create_test_session();
        let diff_source = DiffSource::CommitRange(vec![
            "abc1234567890".to_string(),
            "def4567890123".to_string(),
        ]);

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        // then
        assert!(markdown.contains("Reviewing commits: abc1234, def4567"));
    }

    #[test]
    fn should_include_single_commit_in_markdown() {
        // given
        let session = create_test_session();
        let diff_source = DiffSource::CommitRange(vec!["abc1234567890".to_string()]);

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        // then
        assert!(markdown.contains("Reviewing commit: abc1234"));
    }

    #[test]
    fn should_write_osc52_escape_sequence() {
        // given
        let text = "Hello, World!";
        let mut buffer: Vec<u8> = Vec::new();

        // when
        write_osc52(&mut buffer, text).unwrap();

        // then
        let output = String::from_utf8(buffer).unwrap();
        // OSC 52 format: ESC ] 52 ; c ; <base64> BEL
        assert!(output.starts_with("\x1b]52;c;"));
        assert!(output.ends_with("\x07"));
        // Verify the base64 content
        let base64_content = &output[7..output.len() - 1];
        assert_eq!(BASE64.encode(text), base64_content);
    }

    #[test]
    fn should_encode_empty_string_in_osc52() {
        // given
        let text = "";
        let mut buffer: Vec<u8> = Vec::new();

        // when
        write_osc52(&mut buffer, text).unwrap();

        // then
        let output = String::from_utf8(buffer).unwrap();
        assert_eq!(output, "\x1b]52;c;\x07");
    }

    #[test]
    fn should_encode_unicode_in_osc52() {
        // given
        let text = "こんにちは 🦀";
        let mut buffer: Vec<u8> = Vec::new();

        // when
        write_osc52(&mut buffer, text).unwrap();

        // then
        let output = String::from_utf8(buffer).unwrap();
        let base64_content = &output[7..output.len() - 1];
        // Decode and verify it matches original
        let decoded = String::from_utf8(BASE64.decode(base64_content).unwrap()).unwrap();
        assert_eq!(decoded, text);
    }

    #[test]
    fn should_encode_markdown_content_in_osc52() {
        // given - simulate what would be copied during export
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );
        let mut buffer: Vec<u8> = Vec::new();

        // when
        write_osc52(&mut buffer, &markdown).unwrap();

        // then
        let output = String::from_utf8(buffer).unwrap();
        assert!(output.starts_with("\x1b]52;c;"));
        assert!(output.ends_with("\x07"));
        // Verify we can decode the base64 back to the original markdown
        let base64_content = &output[7..output.len() - 1];
        let decoded = String::from_utf8(BASE64.decode(base64_content).unwrap()).unwrap();
        assert_eq!(decoded, markdown);
    }

    #[test]
    fn should_export_single_line_range_as_single_line() {
        // given - a comment with a single-line range should display as L42, not L42-L42
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);

        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            let range = LineRange::single(42);
            review.add_line_comment(
                42,
                Comment::new_with_range(
                    "Single line comment".to_string(),
                    CommentType::from_id("note"),
                    Some(LineSide::New),
                    range,
                ),
            );
        }
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        // then
        assert!(markdown.contains("`src/main.rs:42`"));
        assert!(!markdown.contains("`src/main.rs:42-42`"));
    }

    #[test]
    fn should_export_line_range_with_start_and_end() {
        // given - a comment spanning multiple lines
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);

        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            let range = LineRange::new(10, 15);
            review.add_line_comment(
                15, // keyed by end line
                Comment::new_with_range(
                    "Multi-line comment".to_string(),
                    CommentType::from_id("issue"),
                    Some(LineSide::New),
                    range,
                ),
            );
        }
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        // then
        assert!(markdown.contains("`src/main.rs:10-15`"));
        assert!(markdown.contains("Multi-line comment"));
    }

    #[test]
    fn should_export_old_side_line_range_with_tilde() {
        // given - a range comment on deleted lines (old side)
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);

        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            let range = LineRange::new(20, 25);
            review.add_line_comment(
                25, // keyed by end line
                Comment::new_with_range(
                    "Deleted lines comment".to_string(),
                    CommentType::from_id("suggestion"),
                    Some(LineSide::Old),
                    range,
                ),
            );
        }
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        // then
        assert!(markdown.contains("`src/main.rs:~20-~25`"));
    }

    #[test]
    fn should_export_single_old_side_line_with_tilde() {
        // given - a single line comment on a deleted line
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);

        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            let range = LineRange::single(30);
            review.add_line_comment(
                30,
                Comment::new_with_range(
                    "Single deleted line".to_string(),
                    CommentType::from_id("note"),
                    Some(LineSide::Old),
                    range,
                ),
            );
        }
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        // then
        assert!(markdown.contains("`src/main.rs:~30`"));
        assert!(!markdown.contains("`src/main.rs:~30-~30`"));
    }

    #[test]
    fn should_handle_comment_without_line_range_field() {
        // given - backward compatibility: comment without line_range uses line number
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);

        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            // Use Comment::new which sets line_range to None
            review.add_line_comment(
                50,
                Comment::new(
                    "Old style comment".to_string(),
                    CommentType::from_id("note"),
                    Some(LineSide::New),
                ),
            );
        }
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        // then
        assert!(markdown.contains("`src/main.rs:50`"));
    }

    #[test]
    fn should_omit_legend_when_legend_is_disabled() {
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;

        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &legend_off(),
            &[],
            None,
        );

        assert!(!markdown.contains("Comment types:"));
        assert!(markdown.contains("[SUGGESTION]"));
        assert!(markdown.contains("[ISSUE]"));
    }

    #[test]
    fn should_only_list_used_comment_types_in_legend() {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.add_file_comment(Comment::new(
                "Great work!".to_string(),
                CommentType::from_id("praise"),
                None,
            ));
        }

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
        );

        assert!(markdown.contains("Comment types: PRAISE (positive feedback)"));
        assert!(!markdown.contains("NOTE"));
        assert!(!markdown.contains("SUGGESTION"));
        assert!(!markdown.contains("ISSUE"));
    }

    #[test]
    fn should_export_none_typed_comments_without_marker_or_legend() {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.add_line_comment(
                42,
                Comment::new(
                    "plain observation".to_string(),
                    CommentType::None,
                    Some(LineSide::New),
                ),
            );
        }

        // Pass the resolved default set (just `None`) to mirror an unconfigured
        // review.
        let none_only = vec![CommentTypeDefinition {
            id: CommentType::NONE_ID.to_string(),
            label: CommentType::NONE_ID.to_string(),
            definition: None,
            color: None,
        }];
        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &none_only,
            &ExportConfig::default(),
            &[],
            None,
        );

        // No `[TYPE]` marker on the comment and no legend line at all.
        assert!(
            !markdown.contains("Comment types:"),
            "unexpected legend:\n{markdown}"
        );
        assert!(
            !markdown.contains("**["),
            "unexpected type marker:\n{markdown}"
        );
        assert!(
            !markdown.contains("[NONE]"),
            "None must not render:\n{markdown}"
        );
        assert!(markdown.contains("`src/main.rs:42`"));
        assert!(markdown.contains("plain observation"));
    }

    fn sample_pr_diff_source() -> DiffSource {
        use crate::app::PullRequestDiffSource;
        use crate::forge::traits::{ForgeRepository, PrSessionKey};
        DiffSource::PullRequest(Box::new(PullRequestDiffSource {
            key: PrSessionKey::new(
                ForgeRepository::github("github.com", "agavra", "tuicr"),
                125,
                "abc1234deadbeef".to_string(),
            ),
            base_sha: "1234567890".to_string(),
            title: "Support reviews".to_string(),
            url: "https://github.com/agavra/tuicr/pull/125".to_string(),
            head_ref_name: "reviews".to_string(),
            base_ref_name: "main".to_string(),
            state: "OPEN".to_string(),
            closed: false,
            merged: false,
        }))
    }

    fn sample_remote_thread(
        id: &str,
        author: &str,
        body: &str,
        line: u32,
        resolved: bool,
    ) -> RemoteReviewThread {
        use crate::forge::remote_comments::{RemoteCommentSide, RemoteReviewComment};
        RemoteReviewThread {
            id: id.to_string(),
            path: "src/lib.rs".to_string(),
            line: Some(line),
            side: RemoteCommentSide::Right,
            is_resolved: resolved,
            is_outdated: false,
            comments: vec![RemoteReviewComment {
                id: format!("{id}-root"),
                author: Some(author.to_string()),
                body: body.to_string(),
                created_at: None,
                in_reply_to: None,
                url: format!("https://github.com/agavra/tuicr/pull/125#discussion_{id}"),
                rest_id: None,
            }],
        }
    }

    #[test]
    fn should_include_unresolved_remote_threads_grouped_by_file_in_pr_export() {
        // given a PR session with one local draft + one unresolved remote
        // thread + one resolved (must be omitted)
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abc1234deadbeef".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        session.add_file(PathBuf::from("src/lib.rs"), FileStatus::Modified, 0);
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/lib.rs")) {
            review.add_line_comment(
                10,
                Comment::new(
                    "Local draft".to_string(),
                    CommentType::from_id("issue"),
                    Some(LineSide::New),
                ),
            );
        }
        let threads = vec![
            sample_remote_thread("a", "alice", "Can this be simpler?", 42, false),
            sample_remote_thread("b", "bob", "Old resolved", 99, true),
        ];

        // when
        let markdown = generate_markdown(
            &session,
            &sample_pr_diff_source(),
            &comment_types(),
            &ExportConfig::default(),
            &threads,
            None,
        );

        // then
        assert!(markdown.contains("Reviewing pull request agavra/tuicr#125"));
        assert!(markdown.contains("## Local tuicr Comments"));
        assert!(markdown.contains("[ISSUE]"));
        assert!(markdown.contains("## Existing GitHub Comments"));
        assert!(markdown.contains("### `src/lib.rs`"));
        assert!(markdown.contains("@alice - Can this be simpler?"));
        // Resolved thread is omitted from export per spec.
        assert!(
            !markdown.contains("Old resolved"),
            "resolved thread leaked into export:\n{markdown}"
        );
    }

    #[test]
    fn should_export_pr_remote_threads_even_when_no_local_drafts() {
        // given a PR session with no local comments but one unresolved remote thread
        let session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abc1234deadbeef".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        let threads = vec![sample_remote_thread("a", "alice", "important", 5, false)];

        // when
        let result = generate_export_content(
            &session,
            &sample_pr_diff_source(),
            &comment_types(),
            &ExportConfig::default(),
            &threads,
            None,
        );

        // then — export succeeds even with no local comments
        let content = result.unwrap();
        assert!(content.contains("## Existing GitHub Comments"));
        assert!(content.contains("@alice - important"));
    }

    /// Blocker 1 regression: a reply/resolve/dismiss made in the TUI
    /// against a durable thread that mirrors a remote-imported
    /// `RemoteReviewThread` must survive into the markdown/agent export —
    /// previously this section only ever read the raw `remote_threads`
    /// DTOs, so any local activity on an already-imported thread was
    /// silently missing from the export entirely.
    #[test]
    fn should_include_a_local_reply_and_status_once_for_a_remote_imported_thread() {
        // given a PR session where the remote thread has already been
        // imported into a durable thread, and a local-only reply plus a
        // local dismiss have been recorded against it (no legacy Comment
        // involved at all — mirrors reply/resolve/dismiss_thread_at_cursor
        // acting on a thread reached only via `RemoteThreadLine`).
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abc1234deadbeef".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        let threads = vec![sample_remote_thread(
            "a",
            "alice",
            "Can this be simpler?",
            42,
            false,
        )];
        session.import_remote_review_threads("github", &threads);
        let thread_id = session
            .find_thread_by_provider("github", "a")
            .expect("thread should have been imported")
            .id()
            .clone();
        session.find_thread_mut(&thread_id).unwrap().thread.reply(
            crate::model::thread::ThreadComment::new(
                crate::model::thread::ThreadAuthor::human("bob"),
                "I'll simplify this".to_string(),
            ),
        );
        session
            .find_thread_mut(&thread_id)
            .unwrap()
            .thread
            .dismiss();

        // when
        let markdown = generate_markdown(
            &session,
            &sample_pr_diff_source(),
            &comment_types(),
            &ExportConfig::default(),
            &threads,
            None,
        );

        // then the remote-authored root still renders from the DTO...
        assert!(markdown.contains("Can this be simpler?"));
        assert!(markdown.contains("@alice"));
        // ...the local-only reply appears exactly once...
        let reply_occurrences = markdown.matches("I'll simplify this").count();
        assert_eq!(
            reply_occurrences, 1,
            "local reply must appear exactly once:\n{markdown}"
        );
        // ...and a local-status marker is visible, distinct from the
        // remote's own (still-`false`) `is_resolved` flag.
        assert!(
            markdown.to_lowercase().contains("locally dismissed"),
            "expected a local dismiss marker:\n{markdown}"
        );
    }

    /// Blocker 1 regression companion: re-fetching/re-importing the same
    /// remote thread must not duplicate a local-only reply in the export
    /// either (mirrors the idempotent-import contract already enforced
    /// for `session.threads` itself).
    #[test]
    fn should_not_duplicate_a_local_reply_in_export_after_reimporting_twice() {
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abc1234deadbeef".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        let threads = vec![sample_remote_thread(
            "a",
            "alice",
            "please review",
            42,
            false,
        )];
        session.import_remote_review_threads("github", &threads);
        let thread_id = session
            .find_thread_by_provider("github", "a")
            .unwrap()
            .id()
            .clone();
        session.find_thread_mut(&thread_id).unwrap().thread.reply(
            crate::model::thread::ThreadComment::new(
                crate::model::thread::ThreadAuthor::human("bob"),
                "keep me exactly once".to_string(),
            ),
        );

        // when re-imported twice (two poll cycles against an unchanged thread)
        session.import_remote_review_threads("github", &threads);
        session.import_remote_review_threads("github", &threads);

        let markdown = generate_markdown(
            &session,
            &sample_pr_diff_source(),
            &comment_types(),
            &ExportConfig::default(),
            &threads,
            None,
        );

        assert_eq!(
            markdown.matches("keep me exactly once").count(),
            1,
            "reimporting twice must not duplicate the local reply:\n{markdown}"
        );
    }

    #[test]
    fn should_not_include_remote_comments_outside_pr_mode_in_export() {
        // given — local working-tree mode + non-empty threads (should be ignored)
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp"),
            "abc".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/lib.rs"), FileStatus::Modified, 0);
        if let Some(r) = session.get_file_mut(&PathBuf::from("src/lib.rs")) {
            r.add_line_comment(
                3,
                Comment::new(
                    "Local".to_string(),
                    CommentType::from_id("note"),
                    Some(LineSide::New),
                ),
            );
        }
        let threads = vec![sample_remote_thread("a", "alice", "ignored", 5, false)];

        // when
        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &threads,
            None,
        );

        // then — the section is omitted in non-PR modes
        assert!(!markdown.contains("Existing GitHub Comments"));
    }

    #[test]
    fn should_only_list_used_custom_types_in_legend() {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.add_file_comment(Comment::new(
                "Needs clarification".to_string(),
                CommentType::from_id("note"),
                None,
            ));
        }

        let custom_types = vec![
            CommentTypeDefinition {
                id: "note".to_string(),
                label: "question".to_string(),
                definition: Some("ask for clarification".to_string()),
                color: None,
            },
            CommentTypeDefinition {
                id: "issue".to_string(),
                label: "issue".to_string(),
                definition: Some("problems to fix".to_string()),
                color: None,
            },
        ];

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &custom_types,
            &ExportConfig::default(),
            &[],
            None,
        );

        assert!(markdown.contains("Comment types: QUESTION (ask for clarification)"));
        assert!(!markdown.contains("ISSUE"));
    }
}
