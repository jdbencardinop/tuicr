//! Non-interactive review session commands.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cli::{AuthorKindArg, LineSideArg, ReviewCommand, ThreadCommand};
use crate::config;
use crate::error::{Result, TuicrError};
use crate::model::comment::{self, CommentLifecycleState};
use crate::model::{
    Anchor, AnchorSide, AnchorState, AnchorTarget, Comment, CommentType, LineRange, LineSide,
    PersistedThread, ReviewSession, ThreadAuthor, ThreadComment, ThreadId, ThreadStatus,
};
use crate::review_store::{
    AddCommentRequest, AddThreadRequest, CommentTarget, ReviewStore, SessionRef, SessionSummary,
};
use crate::slug::Slug;

pub fn run(command: ReviewCommand) -> Result<()> {
    let mut stdout = io::stdout();
    run_with_writer(command, &mut stdout)
}

fn run_with_writer(command: ReviewCommand, out: &mut impl Write) -> Result<()> {
    match command {
        ReviewCommand::List { repo, all } => list_sessions(&repo, all, out),
        ReviewCommand::Add {
            session,
            input,
            repo,
            comment_type,
            file,
            line,
            end_line,
            side,
            username,
            content,
        } => add_comment(
            &session,
            &repo,
            AddCommentOptions {
                input,
                comment_type,
                file,
                line,
                end_line,
                side,
                username,
                content,
            },
            out,
        ),
        ReviewCommand::Comments { session, repo } => show_comments(&session, &repo, out),
        ReviewCommand::Thread { command } => run_thread_command(command, out),
    }
}

fn list_sessions(repo: &Path, all: bool, out: &mut impl Write) -> Result<()> {
    let store = ReviewStore::new();
    let summaries = if all {
        store.list_all_sessions()?
    } else {
        store.list_sessions_for_repo(repo)?
    };
    let output: Vec<_> = summaries
        .into_iter()
        .map(SessionSummaryOutput::from)
        .collect();
    serde_json::to_writer_pretty(&mut *out, &output)?;
    writeln!(out)?;
    Ok(())
}

struct AddCommentOptions {
    input: Option<String>,
    comment_type: String,
    file: Option<PathBuf>,
    line: Option<u32>,
    end_line: Option<u32>,
    side: LineSideArg,
    username: Option<String>,
    content: Option<String>,
}

fn add_comment(
    session: &str,
    repo: &Path,
    options: AddCommentOptions,
    out: &mut impl Write,
) -> Result<()> {
    let store = ReviewStore::new();
    let session_ref = resolve_session_ref(&store, repo, session)?;
    let request_parts = build_add_request_parts(options)?;
    let target = request_parts.target;
    let comment_type = CommentType::from_id(&request_parts.comment_type);
    let author = resolve_cli_author(request_parts.username);
    let comment = store.add_comment(
        &session_ref,
        AddCommentRequest {
            target: target.clone(),
            content: request_parts.content,
            comment_type,
            author,
            commit_id: None,
        },
    )?;
    let output = CommentOutput::from_target(&target, &comment);
    serde_json::to_writer_pretty(&mut *out, &output)?;
    writeln!(out)?;
    Ok(())
}

struct AddRequestParts {
    target: CommentTarget,
    comment_type: String,
    content: String,
    username: Option<String>,
}

/// Resolve the author for a CLI-authored comment.
///
/// Priority: explicit `--username` / JSON `username` ► config `username` ►
/// `Comment::DEFAULT_AUTHOR`. Trims whitespace so `--username " "` doesn't
/// produce an awkward all-whitespace badge.
fn resolve_cli_author(explicit: Option<String>) -> String {
    if let Some(name) = explicit.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return name.to_string();
    }
    if let Ok(outcome) = config::load_config()
        && let Some(name) = outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.username.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    {
        return name.to_string();
    }
    comment::DEFAULT_AUTHOR.to_string()
}

fn build_add_request_parts(options: AddCommentOptions) -> Result<AddRequestParts> {
    let mut comment_type = options.comment_type;
    let mut content = options.content;
    let mut file = options.file;
    let mut line = options.line;
    let mut end_line = options.end_line;
    let mut side = options.side;
    let mut username = options.username;
    let mut target = None;

    if let Some(input) = options.input {
        let payload = parse_add_payload(&read_json_input(&input)?)?;
        if let Some(payload_comment_type) = payload.comment_type {
            comment_type = payload_comment_type;
        }
        if payload.content.is_some() {
            content = payload.content;
        }
        if payload.username.is_some() {
            username = payload.username;
        }
        if let Some(payload_target) = payload.target {
            target = Some(payload_target.into_comment_target()?);
        } else {
            if let Some(payload_file) = payload.file {
                file = Some(payload_file);
            }
            if payload.line.is_some() || payload.start_line.is_some() {
                line = payload.line.or(payload.start_line);
            }
            if let Some(payload_end_line) = payload.end_line {
                end_line = Some(payload_end_line);
            }
            if let Some(payload_side) = payload.side {
                side = parse_line_side(&payload_side)?;
            }
        }
    }

    let content = content.ok_or_else(|| {
        TuicrError::InvalidInput(
            "comment text is required either as COMMENT or JSON field `content`".to_string(),
        )
    })?;
    let target = match target {
        Some(target) => target,
        None => build_comment_target(file, line, end_line, side)?,
    };

    Ok(AddRequestParts {
        target,
        comment_type,
        content,
        username,
    })
}

fn read_json_input(input: &str) -> Result<String> {
    if input == "-" {
        let mut contents = String::new();
        io::stdin().read_to_string(&mut contents)?;
        return Ok(contents);
    }
    if let Some(path) = input.strip_prefix('@') {
        return fs::read_to_string(path).map_err(TuicrError::Io);
    }
    Ok(input.to_string())
}

fn parse_add_payload(input: &str) -> Result<AddCommentPayload> {
    serde_json::from_str(input)
        .map_err(|err| TuicrError::InvalidInput(format!("invalid JSON review payload: {err}")))
}

#[derive(Debug, Deserialize)]
struct AddCommentPayload {
    #[serde(default, alias = "type")]
    comment_type: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    target: Option<JsonCommentTarget>,
    #[serde(default)]
    file: Option<PathBuf>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    start_line: Option<u32>,
    #[serde(default)]
    end_line: Option<u32>,
    #[serde(default)]
    side: Option<String>,
    #[serde(default, alias = "author")]
    username: Option<String>,
    #[serde(default, rename = "author_kind", alias = "authorKind")]
    author_kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JsonCommentTarget {
    #[serde(default, rename = "type", alias = "kind")]
    target_type: Option<String>,
    #[serde(default)]
    file: Option<PathBuf>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    start_line: Option<u32>,
    #[serde(default)]
    end_line: Option<u32>,
    #[serde(default)]
    side: Option<String>,
}

impl JsonCommentTarget {
    fn into_comment_target(self) -> Result<CommentTarget> {
        let side = match self.side {
            Some(side) => parse_line_side(&side)?,
            None => LineSideArg::New,
        };
        let inferred_type = if self.file.is_none() {
            "review"
        } else if self.line.is_some() || self.start_line.is_some() {
            if self.end_line.is_some() {
                "line_range"
            } else {
                "line"
            }
        } else {
            "file"
        };
        let target_type = self
            .target_type
            .unwrap_or_else(|| inferred_type.to_string())
            .replace('-', "_")
            .to_ascii_lowercase();

        match target_type.as_str() {
            "review" => Ok(CommentTarget::Review),
            "file" => Ok(CommentTarget::File {
                path: required_file(self.file, "target.file")?,
            }),
            "line" => Ok(CommentTarget::Line {
                path: required_file(self.file, "target.file")?,
                line: required_line(self.line.or(self.start_line), "target.line")?,
                side: line_side_arg_to_model(side),
            }),
            "line_range" | "range" => Ok(CommentTarget::LineRange {
                path: required_file(self.file, "target.file")?,
                range: LineRange::new(
                    required_line(self.line.or(self.start_line), "target.start_line")?,
                    required_line(self.end_line, "target.end_line")?,
                ),
                side: line_side_arg_to_model(side),
            }),
            other => Err(TuicrError::InvalidInput(format!(
                "unknown JSON target type '{other}'"
            ))),
        }
    }
}

fn required_file(path: Option<PathBuf>, name: &str) -> Result<PathBuf> {
    path.ok_or_else(|| TuicrError::InvalidInput(format!("{name} is required")))
}

fn required_line(line: Option<u32>, name: &str) -> Result<u32> {
    let line = line.ok_or_else(|| TuicrError::InvalidInput(format!("{name} is required")))?;
    validate_line(line, name)?;
    Ok(line)
}

fn parse_line_side(side: &str) -> Result<LineSideArg> {
    match side.to_ascii_lowercase().as_str() {
        "old" => Ok(LineSideArg::Old),
        "new" => Ok(LineSideArg::New),
        other => Err(TuicrError::InvalidInput(format!(
            "unknown side '{other}', expected 'old' or 'new'"
        ))),
    }
}

fn line_side_arg_to_model(side: LineSideArg) -> LineSide {
    match side {
        LineSideArg::Old => LineSide::Old,
        LineSideArg::New => LineSide::New,
    }
}

fn show_comments(session: &str, repo: &Path, out: &mut impl Write) -> Result<()> {
    let store = ReviewStore::new();
    let session_ref = resolve_session_ref(&store, repo, session)?;
    let session = store.get_review(&session_ref)?;
    let comments = collect_comments(&session);
    serde_json::to_writer_pretty(&mut *out, &comments)?;
    writeln!(out)?;
    Ok(())
}

fn resolve_session_ref(store: &ReviewStore, repo: &Path, session: &str) -> Result<SessionRef> {
    let direct_path = PathBuf::from(session);
    if direct_path.exists() || direct_path.is_absolute() || session.ends_with(".json") {
        return Ok(SessionRef::from_path(direct_path));
    }

    // PR sessions are keyed by forge coordinates, not a local checkout, so they
    // resolve from the manifest by slug rather than the per-repo listing.
    if matches!(session.parse::<Slug>(), Ok(Slug::Pr(_))) {
        return match store.resolve_pr_session(session)? {
            Some(session_ref) => Ok(session_ref),
            None => Err(TuicrError::InvalidInput(format!(
                "no PR session found for '{session}'. Run `tuicr review list --all` to see available sessions."
            ))),
        };
    }

    let matches: Vec<_> = store
        .list_sessions_for_repo(repo)?
        .into_iter()
        .filter(|summary| summary.slug == session)
        .collect();
    match matches.as_slice() {
        [summary] => Ok(summary.session_ref.clone()),
        [] => Err(TuicrError::InvalidInput(format!(
            "session '{session}' was not found for repo {}. Run `tuicr review list --repo {}` to see available sessions.",
            repo.display(),
            repo.display()
        ))),
        _ => Err(TuicrError::InvalidInput(format!(
            "session '{session}' is ambiguous for repo {}",
            repo.display()
        ))),
    }
}

fn build_comment_target(
    file: Option<PathBuf>,
    line: Option<u32>,
    end_line: Option<u32>,
    side: LineSideArg,
) -> Result<CommentTarget> {
    let side = match side {
        LineSideArg::Old => LineSide::Old,
        LineSideArg::New => LineSide::New,
    };

    match (file, line, end_line) {
        (None, None, None) => Ok(CommentTarget::Review),
        (Some(path), None, None) => Ok(CommentTarget::File { path }),
        (Some(path), Some(line), None) => {
            validate_line(line, "--line")?;
            Ok(CommentTarget::Line { path, line, side })
        }
        (Some(path), Some(start), Some(end)) => {
            validate_line(start, "--line")?;
            validate_line(end, "--end-line")?;
            Ok(CommentTarget::LineRange {
                path,
                range: LineRange::new(start, end),
                side,
            })
        }
        (None, Some(_), _) => Err(TuicrError::InvalidInput(
            "--line requires --target-file for review comments".to_string(),
        )),
        (None, None, Some(_)) => Err(TuicrError::InvalidInput(
            "--end-line requires --line and --target-file".to_string(),
        )),
        (Some(_), None, Some(_)) => Err(TuicrError::InvalidInput(
            "--end-line requires --line".to_string(),
        )),
    }
}

fn validate_line(line: u32, name: &str) -> Result<()> {
    if line == 0 {
        return Err(TuicrError::InvalidInput(format!(
            "{name} must be greater than zero"
        )));
    }
    Ok(())
}

fn collect_comments(session: &ReviewSession) -> Vec<CommentOutput> {
    let mut comments = Vec::new();
    for comment in &session.review_comments {
        comments.push(CommentOutput::from_parts(
            "review".to_string(),
            None,
            None,
            None,
            None,
            comment,
        ));
    }

    let mut files: Vec<_> = session.files.iter().collect();
    files.sort_by_key(|(path, _)| path.as_os_str().to_os_string());
    for (path, review) in files {
        let path_display = path.to_string_lossy().to_string();
        for comment in &review.file_comments {
            comments.push(CommentOutput::from_parts(
                path_display.clone(),
                Some(path_display.clone()),
                None,
                None,
                None,
                comment,
            ));
        }

        let mut line_comments: Vec<_> = review.line_comments.iter().collect();
        line_comments.sort_by_key(|(line, _)| *line);
        for (line, line_comments) in line_comments {
            for comment in line_comments {
                let (start_line, end_line) = comment
                    .line_range
                    .map(|range| (range.start, range.end))
                    .unwrap_or((*line, *line));
                let location = line_location(&path_display, start_line, end_line, comment.side);
                comments.push(CommentOutput::from_parts(
                    location,
                    Some(path_display.clone()),
                    Some(start_line),
                    Some(end_line),
                    comment.side,
                    comment,
                ));
            }
        }
    }

    comments
}

fn line_location(path: &str, start_line: u32, end_line: u32, side: Option<LineSide>) -> String {
    let line = if start_line == end_line {
        start_line.to_string()
    } else {
        format!("{start_line}-{end_line}")
    };
    match side {
        Some(LineSide::Old) => format!("{path}:{line} [old]"),
        _ => format!("{path}:{line}"),
    }
}

fn target_location(target: &CommentTarget) -> String {
    match target {
        CommentTarget::Review => "review".to_string(),
        CommentTarget::File { path } => path.display().to_string(),
        CommentTarget::Line { path, line, side } => {
            line_location(&path.to_string_lossy(), *line, *line, Some(*side))
        }
        CommentTarget::LineRange { path, range, side } => {
            line_location(&path.to_string_lossy(), range.start, range.end, Some(*side))
        }
    }
}

fn side_id(side: Option<LineSide>) -> Option<&'static str> {
    match side {
        Some(LineSide::Old) => Some("old"),
        Some(LineSide::New) => Some("new"),
        None => None,
    }
}

fn lifecycle_id(state: CommentLifecycleState) -> &'static str {
    match state {
        CommentLifecycleState::LocalDraft => "local_draft",
        CommentLifecycleState::PushedDraft => "pushed_draft",
        CommentLifecycleState::Submitted => "submitted",
    }
}

#[derive(Debug, Serialize)]
struct SessionSummaryOutput {
    slug: String,
    kind: &'static str,
    path: String,
    updated_at: String,
    comment_count: usize,
    reviewed_count: usize,
    file_count: usize,
    anchor: String,
    active: bool,
}

impl From<SessionSummary> for SessionSummaryOutput {
    fn from(summary: SessionSummary) -> Self {
        Self {
            slug: summary.slug,
            kind: summary.kind.id(),
            path: summary.session_ref.path().display().to_string(),
            updated_at: summary.updated_at.to_rfc3339(),
            comment_count: summary.comment_count,
            reviewed_count: summary.reviewed_count,
            file_count: summary.file_count,
            anchor: summary.anchor,
            active: summary.active,
        }
    }
}

#[derive(Debug, Serialize)]
struct CommentOutput {
    id: String,
    location: String,
    path: Option<String>,
    start_line: Option<u32>,
    end_line: Option<u32>,
    side: Option<&'static str>,
    comment_type: String,
    lifecycle_state: &'static str,
    author: String,
    created_at: String,
    content: String,
}

impl CommentOutput {
    fn from_target(target: &CommentTarget, comment: &Comment) -> Self {
        let (path, start_line, end_line, side) = match target {
            CommentTarget::Review => (None, None, None, None),
            CommentTarget::File { path } => (Some(path.display().to_string()), None, None, None),
            CommentTarget::Line { path, line, side } => (
                Some(path.display().to_string()),
                Some(*line),
                Some(*line),
                Some(*side),
            ),
            CommentTarget::LineRange { path, range, side } => (
                Some(path.display().to_string()),
                Some(range.start),
                Some(range.end),
                Some(*side),
            ),
        };
        Self::from_parts(
            target_location(target),
            path,
            start_line,
            end_line,
            side,
            comment,
        )
    }

    fn from_parts(
        location: String,
        path: Option<String>,
        start_line: Option<u32>,
        end_line: Option<u32>,
        side: Option<LineSide>,
        comment: &Comment,
    ) -> Self {
        Self {
            id: comment.id.clone(),
            location,
            path,
            start_line,
            end_line,
            side: side_id(side),
            comment_type: comment.comment_type.id().to_string(),
            lifecycle_state: lifecycle_id(comment.lifecycle_state),
            author: comment.author.clone(),
            created_at: comment.created_at.to_rfc3339(),
            content: comment.content.clone(),
        }
    }
}

// ---------- Thread commands ----------

fn run_thread_command(command: ThreadCommand, out: &mut impl Write) -> Result<()> {
    match command {
        ThreadCommand::List { session, repo } => list_threads(&session, &repo, out),
        ThreadCommand::Show {
            session,
            repo,
            thread_id,
        } => show_thread(&session, &repo, &thread_id, out),
        ThreadCommand::Add {
            session,
            repo,
            input,
            file,
            line,
            end_line,
            side,
            author,
            author_kind,
            content,
        } => add_thread(
            &session,
            &repo,
            AddThreadOptions {
                input,
                file,
                line,
                end_line,
                side,
                author,
                author_kind,
                content,
            },
            out,
        ),
        ThreadCommand::Reply {
            session,
            repo,
            thread_id,
            input,
            author,
            author_kind,
            content,
        } => reply_to_thread(
            &session,
            &repo,
            &thread_id,
            ReplyOptions {
                input,
                author,
                author_kind,
                content,
            },
            out,
        ),
        ThreadCommand::Resolve {
            session,
            repo,
            thread_id,
        } => update_thread_status(&session, &repo, &thread_id, ThreadStatusOp::Resolve, out),
        ThreadCommand::Reopen {
            session,
            repo,
            thread_id,
        } => update_thread_status(&session, &repo, &thread_id, ThreadStatusOp::Reopen, out),
        ThreadCommand::Dismiss {
            session,
            repo,
            thread_id,
        } => update_thread_status(&session, &repo, &thread_id, ThreadStatusOp::Dismiss, out),
    }
}

fn list_threads(session: &str, repo: &Path, out: &mut impl Write) -> Result<()> {
    let store = ReviewStore::new();
    let session_ref = resolve_session_ref(&store, repo, session)?;
    let threads = store.list_threads(&session_ref)?;
    let output: Vec<_> = threads.iter().map(ThreadOutput::from).collect();
    serde_json::to_writer_pretty(&mut *out, &output)?;
    writeln!(out)?;
    Ok(())
}

fn show_thread(session: &str, repo: &Path, thread_id: &str, out: &mut impl Write) -> Result<()> {
    let store = ReviewStore::new();
    let session_ref = resolve_session_ref(&store, repo, session)?;
    let id = parse_thread_id(thread_id)?;
    let thread = store
        .get_thread(&session_ref, &id)?
        .ok_or_else(|| TuicrError::InvalidInput(format!("thread '{thread_id}' not found")))?;
    let output = ThreadOutput::from(&thread);
    serde_json::to_writer_pretty(&mut *out, &output)?;
    writeln!(out)?;
    Ok(())
}

struct AddThreadOptions {
    input: Option<String>,
    file: Option<PathBuf>,
    line: Option<u32>,
    end_line: Option<u32>,
    side: LineSideArg,
    author: Option<String>,
    author_kind: AuthorKindArg,
    content: Option<String>,
}

fn add_thread(
    session: &str,
    repo: &Path,
    options: AddThreadOptions,
    out: &mut impl Write,
) -> Result<()> {
    let store = ReviewStore::new();
    let session_ref = resolve_session_ref(&store, repo, session)?;

    let mut file = options.file;
    let mut line = options.line;
    let mut end_line = options.end_line;
    let mut side = options.side;
    let mut author = options.author;
    let mut author_kind = options.author_kind;
    let mut content = options.content;
    let mut target = None;

    if let Some(input) = options.input {
        let payload = parse_add_payload(&read_json_input(&input)?)?;
        if payload.content.is_some() {
            content = payload.content;
        }
        if payload.username.is_some() {
            author = payload.username;
        }
        if let Some(kind) = payload.author_kind.as_deref() {
            author_kind = parse_author_kind(kind)?;
        }
        if let Some(payload_target) = payload.target {
            target = Some(payload_target.into_comment_target()?);
        } else {
            if let Some(payload_file) = payload.file {
                file = Some(payload_file);
            }
            if payload.line.is_some() || payload.start_line.is_some() {
                line = payload.line.or(payload.start_line);
            }
            if let Some(payload_end_line) = payload.end_line {
                end_line = Some(payload_end_line);
            }
            if let Some(payload_side) = payload.side {
                side = parse_line_side(&payload_side)?;
            }
        }
    }

    let content = content.ok_or_else(|| {
        TuicrError::InvalidInput(
            "thread body is required either as COMMENT or JSON field `content`".to_string(),
        )
    })?;
    let target = match target {
        Some(target) => target,
        None => build_comment_target(file, line, end_line, side)?,
    };
    let thread_author = resolve_cli_thread_author(author, author_kind);

    let thread = store.add_thread(
        &session_ref,
        AddThreadRequest {
            target,
            body: content,
            author: thread_author,
        },
    )?;
    let output = ThreadOutput::from(&thread);
    serde_json::to_writer_pretty(&mut *out, &output)?;
    writeln!(out)?;
    Ok(())
}

struct ReplyOptions {
    input: Option<String>,
    author: Option<String>,
    author_kind: AuthorKindArg,
    content: Option<String>,
}

fn reply_to_thread(
    session: &str,
    repo: &Path,
    thread_id: &str,
    options: ReplyOptions,
    out: &mut impl Write,
) -> Result<()> {
    let store = ReviewStore::new();
    let session_ref = resolve_session_ref(&store, repo, session)?;

    let mut author = options.author;
    let mut author_kind = options.author_kind;
    let mut content = options.content;

    if let Some(input) = options.input {
        let payload = parse_reply_payload(&read_json_input(&input)?)?;
        if payload.content.is_some() {
            content = payload.content;
        }
        if payload.username.is_some() {
            author = payload.username;
        }
        if let Some(kind) = payload.author_kind.as_deref() {
            author_kind = parse_author_kind(kind)?;
        }
    }

    let content = content.ok_or_else(|| {
        TuicrError::InvalidInput(
            "reply body is required either as COMMENT or JSON field `content`".to_string(),
        )
    })?;
    let thread_author = resolve_cli_thread_author(author, author_kind);
    let id = parse_thread_id(thread_id)?;

    let thread = store.reply_to_thread(&session_ref, &id, thread_author, content)?;
    let output = ThreadOutput::from(&thread);
    serde_json::to_writer_pretty(&mut *out, &output)?;
    writeln!(out)?;
    Ok(())
}

#[derive(Clone, Copy)]
enum ThreadStatusOp {
    Resolve,
    Reopen,
    Dismiss,
}

fn update_thread_status(
    session: &str,
    repo: &Path,
    thread_id: &str,
    op: ThreadStatusOp,
    out: &mut impl Write,
) -> Result<()> {
    let store = ReviewStore::new();
    let session_ref = resolve_session_ref(&store, repo, session)?;
    let id = parse_thread_id(thread_id)?;

    match op {
        ThreadStatusOp::Resolve => {
            store.resolve_thread(&session_ref, &id)?;
        }
        ThreadStatusOp::Reopen => {
            store.reopen_thread(&session_ref, &id)?;
        }
        ThreadStatusOp::Dismiss => {
            store.dismiss_thread(&session_ref, &id)?;
        }
    }

    let thread = store
        .get_thread(&session_ref, &id)?
        .ok_or_else(|| TuicrError::InvalidInput(format!("thread '{thread_id}' not found")))?;
    let output = ThreadOutput::from(&thread);
    serde_json::to_writer_pretty(&mut *out, &output)?;
    writeln!(out)?;
    Ok(())
}

/// Resolve the author for a CLI-authored thread comment/reply.
///
/// Priority: explicit `--author` / JSON `username`/`author` ► config
/// `username` ► `Comment::DEFAULT_AUTHOR`. `--author-kind` (default
/// `human`) picks the [`ThreadAuthor`] variant; provider-imported threads
/// use [`ThreadAuthor::remote`] instead, which this CLI path never
/// produces (that's reserved for a future provider adapter).
fn resolve_cli_thread_author(explicit: Option<String>, kind: AuthorKindArg) -> ThreadAuthor {
    let name = resolve_cli_author(explicit);
    match kind {
        AuthorKindArg::Human => ThreadAuthor::human(name),
        AuthorKindArg::Agent => ThreadAuthor::agent(name),
    }
}

fn parse_thread_id(raw: &str) -> Result<ThreadId> {
    serde_json::from_value(serde_json::Value::String(raw.to_string()))
        .map_err(|err| TuicrError::InvalidInput(format!("invalid thread id '{raw}': {err}")))
}

fn parse_author_kind(kind: &str) -> Result<AuthorKindArg> {
    match kind.to_ascii_lowercase().as_str() {
        "human" => Ok(AuthorKindArg::Human),
        "agent" => Ok(AuthorKindArg::Agent),
        other => Err(TuicrError::InvalidInput(format!(
            "unknown author kind '{other}', expected 'human' or 'agent'"
        ))),
    }
}

fn parse_reply_payload(input: &str) -> Result<ReplyPayload> {
    serde_json::from_str(input)
        .map_err(|err| TuicrError::InvalidInput(format!("invalid JSON reply payload: {err}")))
}

#[derive(Debug, Deserialize)]
struct ReplyPayload {
    #[serde(default)]
    content: Option<String>,
    #[serde(default, alias = "author")]
    username: Option<String>,
    #[serde(default, rename = "author_kind", alias = "authorKind")]
    author_kind: Option<String>,
}

#[derive(Debug, Serialize)]
struct ThreadAuthorOutput {
    kind: &'static str,
    name: String,
    provider_id: Option<String>,
}

impl From<&ThreadAuthor> for ThreadAuthorOutput {
    fn from(author: &ThreadAuthor) -> Self {
        Self {
            kind: if author.is_human() {
                "human"
            } else if author.is_agent() {
                "agent"
            } else {
                "remote"
            },
            name: author.name.clone(),
            provider_id: author.provider_id.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct ThreadCommentOutput {
    id: String,
    author: ThreadAuthorOutput,
    body: String,
    created_at: String,
    updated_at: Option<String>,
}

impl From<&ThreadComment> for ThreadCommentOutput {
    fn from(comment: &ThreadComment) -> Self {
        Self {
            id: comment.id().as_str().to_string(),
            author: ThreadAuthorOutput::from(&comment.author),
            body: comment.body.clone(),
            created_at: comment.created_at.to_rfc3339(),
            updated_at: comment.updated_at.map(|dt| dt.to_rfc3339()),
        }
    }
}

#[derive(Debug, Serialize)]
struct ThreadAnchorOutput {
    kind: &'static str,
    path: Option<String>,
    side: Option<&'static str>,
    line: Option<u32>,
    end_line: Option<u32>,
    state: &'static str,
}

fn anchor_side_id(side: AnchorSide) -> &'static str {
    match side {
        AnchorSide::Old => "old",
        AnchorSide::New => "new",
        AnchorSide::Both => "both",
    }
}

fn anchor_state_id(state: AnchorState) -> &'static str {
    match state {
        AnchorState::Current => "current",
        AnchorState::Stale => "stale",
        AnchorState::Ambiguous => "ambiguous",
    }
}

fn thread_status_id(status: ThreadStatus) -> &'static str {
    match status {
        ThreadStatus::Open => "open",
        ThreadStatus::Resolved => "resolved",
        ThreadStatus::Stale => "stale",
        ThreadStatus::Ambiguous => "ambiguous",
        ThreadStatus::Dismissed => "dismissed",
    }
}

impl From<&Anchor> for ThreadAnchorOutput {
    fn from(anchor: &Anchor) -> Self {
        let state = anchor_state_id(anchor.state());
        match anchor.target() {
            AnchorTarget::Review => Self {
                kind: "review",
                path: None,
                side: None,
                line: None,
                end_line: None,
                state,
            },
            AnchorTarget::File { path } => Self {
                kind: "file",
                path: Some(path.clone()),
                side: None,
                line: None,
                end_line: None,
                state,
            },
            AnchorTarget::Line { path, side, line } => Self {
                kind: "line",
                path: Some(path.clone()),
                side: Some(anchor_side_id(*side)),
                line: Some(*line),
                end_line: None,
                state,
            },
            AnchorTarget::Range {
                path,
                side,
                start,
                end,
            } => Self {
                kind: "range",
                path: Some(path.clone()),
                side: Some(anchor_side_id(*side)),
                line: Some(*start),
                end_line: Some(*end),
                state,
            },
        }
    }
}

#[derive(Debug, Serialize)]
struct ThreadOutput {
    id: String,
    status: &'static str,
    anchor: ThreadAnchorOutput,
    comments: Vec<ThreadCommentOutput>,
    provider_mappings: std::collections::HashMap<String, serde_json::Value>,
}

impl From<&PersistedThread> for ThreadOutput {
    fn from(persisted: &PersistedThread) -> Self {
        Self {
            id: persisted.id().as_str().to_string(),
            status: thread_status_id(persisted.thread.status()),
            anchor: ThreadAnchorOutput::from(persisted.thread.anchor()),
            comments: persisted
                .thread
                .comments()
                .iter()
                .map(ThreadCommentOutput::from)
                .collect(),
            provider_mappings: persisted.provider_mappings.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    use crate::model::{FileStatus, SessionDiffSource};

    fn test_session(repo_path: PathBuf) -> ReviewSession {
        let mut session = ReviewSession::new(
            repo_path,
            "abc1234".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        session
    }

    #[test]
    fn should_build_review_comment_target_by_default() {
        let target = build_comment_target(None, None, None, LineSideArg::New).unwrap();
        assert!(matches!(target, CommentTarget::Review));
    }

    #[test]
    fn should_build_line_range_comment_target() {
        let target = build_comment_target(
            Some(PathBuf::from("src/main.rs")),
            Some(12),
            Some(10),
            LineSideArg::Old,
        )
        .unwrap();

        assert!(matches!(
            target,
            CommentTarget::LineRange {
                range: LineRange { start: 10, end: 12 },
                side: LineSide::Old,
                ..
            }
        ));
    }

    #[test]
    fn should_reject_zero_line() {
        let err = build_comment_target(
            Some(PathBuf::from("src/main.rs")),
            Some(0),
            None,
            LineSideArg::New,
        )
        .unwrap_err();
        assert!(matches!(err, TuicrError::InvalidInput(_)));
    }

    #[test]
    fn should_build_add_request_from_flat_json_payload() {
        let parts = build_add_request_parts(AddCommentOptions {
            input: Some(
                r#"{"file":"src/main.rs","line":42,"side":"old","type":"issue","content":"fix it"}"#
                    .to_string(),
            ),
            comment_type: "note".to_string(),
            file: None,
            line: None,
            end_line: None,
            side: LineSideArg::New,
            username: None,
            content: None,
        })
        .unwrap();

        assert_eq!(parts.comment_type, "issue");
        assert_eq!(parts.content, "fix it");
        assert!(matches!(
            parts.target,
            CommentTarget::Line {
                path,
                line: 42,
                side: LineSide::Old,
            } if path.as_path() == Path::new("src/main.rs")
        ));
    }

    #[test]
    fn should_build_add_request_from_nested_json_payload() {
        let parts = build_add_request_parts(AddCommentOptions {
            input: Some(
                r#"{"comment_type":"suggestion","content":"collapse this","target":{"type":"line_range","file":"src/main.rs","start_line":5,"end_line":7}}"#
                    .to_string(),
            ),
            comment_type: "note".to_string(),
            file: None,
            line: None,
            end_line: None,
            side: LineSideArg::New,
            username: None,
            content: None,
        })
        .unwrap();

        assert_eq!(parts.comment_type, "suggestion");
        assert!(matches!(
            parts.target,
            CommentTarget::LineRange {
                range: LineRange { start: 5, end: 7 },
                side: LineSide::New,
                ..
            }
        ));
    }

    fn save_pr_session(store: &ReviewStore) -> SessionRef {
        use crate::forge::traits::{ForgeRepository, PrSessionKey};

        let key = PrSessionKey::new(
            ForgeRepository::github("github.com", "slatedb", "slatedb"),
            1745,
            "43e3566924690c06a45b2177b4dd2df59a0f09c6".to_string(),
        );
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/slatedb/slatedb"),
            key.head_sha.clone(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        session.pr_session_key = Some(key);
        store.save_review(&session).unwrap()
    }

    #[test]
    fn should_find_pr_session_by_repo_coordinate() {
        let temp = tempdir().unwrap();
        let store = ReviewStore::with_reviews_dir(temp.path().join("reviews"));
        let session_ref = save_pr_session(&store);

        // A bare repo coordinate surfaces the PR session and emits its slug.
        let listed = store
            .list_sessions_for_repo(Path::new("slatedb/slatedb"))
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].slug, "gh:slatedb/slatedb/pr/1745");
        assert_eq!(listed[0].kind, crate::review_store::SessionKind::Pr);

        // The emitted slug resolves the same way regardless of --repo.
        let resolved =
            resolve_session_ref(&store, Path::new("slatedb/slatedb"), &listed[0].slug).unwrap();
        assert_eq!(resolved, session_ref);
    }

    #[test]
    fn should_match_pr_session_via_forge_repo_path_coordinate() {
        let temp = tempdir().unwrap();
        let store = ReviewStore::with_reviews_dir(temp.path().join("reviews"));
        save_pr_session(&store);

        // The `forge:host/owner/repo` form (as stored on disk) also resolves.
        let listed = store
            .list_sessions_for_repo(Path::new("forge:github.com/slatedb/slatedb"))
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].slug, "gh:slatedb/slatedb/pr/1745");
    }

    #[test]
    fn should_not_match_pr_session_for_unrelated_repo() {
        let temp = tempdir().unwrap();
        let store = ReviewStore::with_reviews_dir(temp.path().join("reviews"));
        save_pr_session(&store);

        assert!(
            store
                .list_sessions_for_repo(Path::new("other/project"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn should_list_pr_session_in_list_all() {
        let temp = tempdir().unwrap();
        let store = ReviewStore::with_reviews_dir(temp.path().join("reviews"));
        save_pr_session(&store);

        let all = store.list_all_sessions().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].slug, "gh:slatedb/slatedb/pr/1745");
    }

    #[test]
    fn should_resolve_pr_session_by_slug_without_repo() {
        let temp = tempdir().unwrap();
        let store = ReviewStore::with_reviews_dir(temp.path().join("reviews"));
        let session_ref = save_pr_session(&store);

        // PR slugs are self-contained: `--repo` is irrelevant.
        let resolved =
            resolve_session_ref(&store, Path::new("."), "gh:slatedb/slatedb/pr/1745").unwrap();
        assert_eq!(resolved, session_ref);
    }

    #[test]
    fn should_error_for_unknown_pr_slug() {
        let temp = tempdir().unwrap();
        let reviews = temp.path().join("reviews");
        let store = ReviewStore::with_reviews_dir(&reviews);
        let err = resolve_session_ref(&store, Path::new("."), "gh:nope/nope/pr/9999").unwrap_err();
        assert!(matches!(err, TuicrError::InvalidInput(_)));
    }

    #[test]
    fn should_list_add_and_show_comments() {
        let temp = tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let reviews = temp.path().join("reviews");
        let store = ReviewStore::with_reviews_dir(&reviews);
        let session = test_session(repo.clone());
        let session_ref = store.save_review(&session).unwrap();

        let mut out = Vec::new();
        let sessions = store.list_sessions_for_repo(&repo).unwrap();
        assert_eq!(sessions.len(), 1);
        let slug = sessions[0].slug.clone();

        let resolved = resolve_session_ref(&store, &repo, &slug).unwrap();
        assert_eq!(resolved, session_ref);

        let comment = store
            .add_comment(
                &resolved,
                AddCommentRequest {
                    target: CommentTarget::Line {
                        path: PathBuf::from("src/main.rs"),
                        line: 42,
                        side: LineSide::New,
                    },
                    content: "check this".to_string(),
                    comment_type: CommentType::from_id("issue"),
                    author: "Claude Sonnet 5".to_string(),
                    commit_id: None,
                },
            )
            .unwrap();

        let loaded = store.get_review(&session_ref).unwrap();
        let comments = collect_comments(&loaded);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].id, comment.id);
        assert_eq!(comments[0].location, "src/main.rs:42");
        assert_eq!(comments[0].comment_type, "issue");
        assert_eq!(comments[0].author, "Claude Sonnet 5");

        show_comments(&session_ref.path().display().to_string(), &repo, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value[0]["comment_type"], "issue");
        assert_eq!(value[0]["location"], "src/main.rs:42");
        assert_eq!(value[0]["author"], "Claude Sonnet 5");
        assert_eq!(value[0]["content"], "check this");
    }

    // ---- Thread commands ----

    fn setup_thread_store() -> (tempfile::TempDir, PathBuf, ReviewStore, SessionRef) {
        let temp = tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let reviews = temp.path().join("reviews");
        let store = ReviewStore::with_reviews_dir(&reviews);
        let session = test_session(repo.clone());
        let session_ref = store.save_review(&session).unwrap();
        (temp, repo, store, session_ref)
    }

    #[test]
    fn should_list_and_show_threads_via_cli_json_output() {
        let (_temp, repo, store, session_ref) = setup_thread_store();

        let thread = store
            .add_thread(
                &session_ref,
                AddThreadRequest {
                    target: CommentTarget::Line {
                        path: PathBuf::from("src/main.rs"),
                        line: 7,
                        side: LineSide::New,
                    },
                    body: "please add a test".to_string(),
                    author: ThreadAuthor::human("Ada"),
                },
            )
            .unwrap();

        let mut out = Vec::new();
        list_threads(&session_ref.path().display().to_string(), &repo, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value.as_array().unwrap().len(), 1);
        assert_eq!(value[0]["id"], thread.id().as_str());
        assert_eq!(value[0]["status"], "open");
        assert_eq!(value[0]["anchor"]["kind"], "line");
        assert_eq!(value[0]["anchor"]["path"], "src/main.rs");
        assert_eq!(value[0]["anchor"]["line"], 7);
        assert_eq!(value[0]["anchor"]["side"], "new");
        assert_eq!(value[0]["anchor"]["state"], "current");
        assert_eq!(value[0]["comments"][0]["author"]["kind"], "human");
        assert_eq!(value[0]["comments"][0]["author"]["name"], "Ada");
        assert_eq!(value[0]["comments"][0]["body"], "please add a test");

        let mut show_out = Vec::new();
        show_thread(
            &session_ref.path().display().to_string(),
            &repo,
            thread.id().as_str(),
            &mut show_out,
        )
        .unwrap();
        let show_text = String::from_utf8(show_out).unwrap();
        let show_value: serde_json::Value = serde_json::from_str(&show_text).unwrap();
        assert_eq!(show_value["id"], thread.id().as_str());
        assert_eq!(show_value["comments"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn should_error_when_showing_unknown_thread_id() {
        let (_temp, repo, _store, session_ref) = setup_thread_store();
        let mut out = Vec::new();
        let unknown_id = crate::model::ThreadId::new();
        let err = show_thread(
            &session_ref.path().display().to_string(),
            &repo,
            unknown_id.as_str(),
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn should_show_review_comments_and_threads_side_by_side_after_migration() {
        // A legacy (v1.3) session's `review comments` keeps surfacing its
        // original comments unchanged, while `review thread list` surfaces
        // the same comments migrated into threads once loaded, from the
        // same persisted session — neither view silently drops the other.
        let temp = tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let reviews_dir = temp.path().join("reviews");

        let mut session = test_session(repo.clone());
        session.version = "1.3".to_string();
        session.review_comments.push(Comment::new(
            "legacy review comment".to_string(),
            CommentType::None,
            None,
        ));
        let session_ref = crate::persistence::storage::save_session_in_dir(&session, &reviews_dir)
            .map(SessionRef::from_path)
            .unwrap();

        let mut comments_out = Vec::new();
        show_comments(
            &session_ref.path().display().to_string(),
            &repo,
            &mut comments_out,
        )
        .unwrap();
        let comments_value: serde_json::Value =
            serde_json::from_str(&String::from_utf8(comments_out).unwrap()).unwrap();
        assert_eq!(comments_value[0]["content"], "legacy review comment");

        let mut threads_out = Vec::new();
        list_threads(
            &session_ref.path().display().to_string(),
            &repo,
            &mut threads_out,
        )
        .unwrap();
        let threads_value: serde_json::Value =
            serde_json::from_str(&String::from_utf8(threads_out).unwrap()).unwrap();
        assert_eq!(threads_value.as_array().unwrap().len(), 1);
        assert_eq!(
            threads_value[0]["comments"][0]["body"],
            "legacy review comment"
        );
        assert_eq!(
            threads_value[0]["comments"][0]["author"]["name"],
            comment::DEFAULT_AUTHOR
        );

        // The on-disk v1.3 file is untouched by the read-only listing above;
        // re-fetching comments still works from the same unmigrated file.
        let raw_contents = std::fs::read_to_string(session_ref.path()).unwrap();
        let on_disk: ReviewSession = serde_json::from_str(&raw_contents).unwrap();
        assert_eq!(on_disk.version, "1.3");
    }

    #[test]
    fn should_resolve_cli_thread_author_human_and_agent() {
        let human = resolve_cli_thread_author(Some("Ada".to_string()), AuthorKindArg::Human);
        assert!(human.is_human());
        assert_eq!(human.name, "Ada");

        let agent = resolve_cli_thread_author(Some("Claude".to_string()), AuthorKindArg::Agent);
        assert!(agent.is_agent());
        assert_eq!(agent.name, "Claude");
    }

    #[test]
    fn should_parse_author_kind_case_insensitively() {
        assert_eq!(parse_author_kind("Human").unwrap(), AuthorKindArg::Human);
        assert_eq!(parse_author_kind("AGENT").unwrap(), AuthorKindArg::Agent);
        assert!(parse_author_kind("bot").is_err());
    }
}
