use super::*;

impl App {
    /// Drive `:submit*` preflight: walk every local-draft comment in the
    /// current PR session, map each one against the displayed diff, bucket
    /// the results, and transition into the resolver (when there are
    /// unmappable comments) or the final-confirmation modal.
    ///
    /// PR 5 does not call the network; `[y]` in the confirmation modal
    /// stubs a "PR 6 will wire the network call" info message.
    pub fn start_submit(&mut self, event: crate::forge::submit::SubmitEvent) {
        self.start_submit_with(event, false);
    }

    pub(in crate::app) fn build_gitlab_publication(
        session: &crate::model::review::ReviewSession,
        mappable: &[crate::forge::submit::InlineComment],
    ) -> Result<(
        crate::model::review::ReviewSession,
        crate::forge::dryrun::DryRunPlan,
        Vec<String>,
    )> {
        use crate::forge::dryrun::{OperationKind, plan_publication};

        let selected: std::collections::HashSet<&str> = mappable
            .iter()
            .map(|item| item.comment_id.as_str())
            .collect();
        let mut publication = session.clone();
        let mut legacy_fallback_ids = Vec::new();
        publication.threads.retain_mut(|persisted| {
            let mapped = persisted.provider_mapping("gitlab").is_some();
            let root_selected = persisted
                .thread
                .root()
                .is_some_and(|root| selected.contains(root.id().as_str()));
            let selected_reply_ids: Vec<String> = persisted
                .thread
                .replies()
                .filter(|reply| selected.contains(reply.id().as_str()))
                .map(|reply| reply.id().as_str().to_string())
                .collect();
            if persisted.is_legacy_mirror()
                && !mapped
                && !root_selected
                && !selected_reply_ids.is_empty()
            {
                legacy_fallback_ids.extend(selected_reply_ids);
                return false;
            }
            let keep = mapped || !persisted.is_legacy_mirror() || root_selected;
            if keep {
                persisted.thread.retain_replies(|reply| {
                    !session.is_legacy_comment_id(reply.id().as_str())
                        || selected.contains(reply.id().as_str())
                });
            }
            keep
        });

        let capabilities = crate::forge::capabilities::gitlab();
        let plan = plan_publication(&publication, &capabilities, None);

        for comment_id in selected {
            if legacy_fallback_ids.iter().any(|id| id == comment_id) {
                continue;
            }
            let Some((thread, index)) = publication.threads.iter().find_map(|thread| {
                thread
                    .thread
                    .comments()
                    .iter()
                    .position(|comment| comment.id().as_str() == comment_id)
                    .map(|index| (thread, index))
            }) else {
                return Err(TuicrError::Forge(format!(
                    "selected comment `{comment_id}` has no publishable durable thread"
                )));
            };
            let covered = plan
                .operations
                .iter()
                .filter(|operation| operation.thread_id.as_deref() == Some(thread.id().as_str()))
                .filter(|operation| match &operation.op {
                    OperationKind::CreateThread => index == 0,
                    OperationKind::Reply {
                        comment_id: planned_id,
                    } => index > 0 && planned_id == comment_id,
                    _ => false,
                })
                .count();
            if covered != 1 {
                return Err(TuicrError::Forge(format!(
                    "selected comment `{comment_id}` is covered by {covered} durable operations"
                )));
            }
        }

        Ok((publication, plan, legacy_fallback_ids))
    }

    fn submit_result_is_stale(
        &self,
        in_flight: &SubmitInFlightState,
        repository: &crate::forge::traits::ForgeRepository,
        pr_number: u64,
        head_sha: &str,
    ) -> bool {
        let event_mismatch = &in_flight.repository != repository
            || in_flight.pr_number != pr_number
            || in_flight.head_sha_snapshot != head_sha;
        let current_mismatch = match &self.diff_source {
            DiffSource::PullRequest(pr) => {
                &pr.key.repository != repository
                    || pr.key.number != pr_number
                    || pr.key.head_sha != in_flight.pr_head_snapshot
                    || self
                        .current_pr_head
                        .as_deref()
                        .is_some_and(|current| current != in_flight.pr_head_snapshot)
            }
            _ => true,
        };
        event_mismatch || current_mismatch
    }

    fn checkpoint_publication_mapping(
        identity: &crate::model::review::ReviewSession,
        published: &crate::model::review::ReviewSession,
        operation: &crate::forge::dryrun::PlannedOperation,
    ) -> Result<()> {
        let Some(thread_id) = operation.thread_id.as_deref() else {
            return Ok(());
        };
        let published_thread = published
            .threads
            .iter()
            .find(|thread| thread.id().as_str() == thread_id)
            .ok_or_else(|| {
                TuicrError::Forge(format!(
                    "published thread `{thread_id}` disappeared before checkpoint"
                ))
            })?;
        let reply_mapping = match &operation.op {
            crate::forge::dryrun::OperationKind::Reply { comment_id } => published_thread
                .published_reply_id("gitlab", comment_id)
                .map(|remote_id| (comment_id.clone(), remote_id.to_string())),
            _ => None,
        };
        let completed_comment_id = match &operation.op {
            crate::forge::dryrun::OperationKind::CreateThread => published_thread
                .thread
                .root()
                .map(|root| root.id().as_str().to_string()),
            crate::forge::dryrun::OperationKind::Reply { comment_id } => Some(comment_id.clone()),
            _ => None,
        };
        let mappings: Vec<(String, serde_json::Value)> = Self::publication_mapping_keys(operation)
            .into_iter()
            .filter_map(|key| {
                published_thread
                    .provider_mappings
                    .get(&key)
                    .cloned()
                    .map(|mapping| (key, mapping))
            })
            .collect();

        crate::persistence::storage::save_session_by_identity(identity, |persisted| {
            let mut session = persisted.unwrap_or_else(|| identity.clone());
            let thread = session
                .threads
                .iter_mut()
                .find(|thread| thread.id().as_str() == thread_id)
                .ok_or_else(|| {
                    TuicrError::Forge(format!(
                        "cannot checkpoint mapping for missing thread `{thread_id}`"
                    ))
                })?;
            if let Some((comment_id, remote_id)) = reply_mapping {
                thread.record_published_reply("gitlab", comment_id, remote_id);
            } else {
                for (key, mapping) in mappings {
                    thread.provider_mappings.insert(key, mapping);
                }
                if let Some(comment_id) = completed_comment_id.as_deref() {
                    Self::mark_persisted_comment_submitted(&mut session, comment_id);
                }
            }
            session.updated_at = chrono::Utc::now();
            Ok((session, ()))
        })?;
        Ok(())
    }

    fn mark_persisted_comment_submitted(
        session: &mut crate::model::review::ReviewSession,
        comment_id: &str,
    ) {
        use crate::model::comment::CommentLifecycleState;

        let mark = |comment: &mut crate::model::comment::Comment| {
            if comment.id == comment_id {
                comment.lifecycle_state = CommentLifecycleState::Submitted;
            }
        };
        for comment in &mut session.review_comments {
            mark(comment);
        }
        for review in session.files.values_mut() {
            for comment in &mut review.file_comments {
                mark(comment);
            }
            for comments in review.line_comments.values_mut() {
                for comment in comments {
                    mark(comment);
                }
            }
        }
    }

    fn recover_publication_checkpoints(
        identity: &crate::model::review::ReviewSession,
        published: &crate::model::review::ReviewSession,
        report: &crate::forge::publish::PublishReport,
    ) -> Result<()> {
        for operation in report.completed() {
            Self::checkpoint_publication_mapping(identity, published, operation)?;
        }
        Ok(())
    }

    fn publication_mapping_keys(operation: &crate::forge::dryrun::PlannedOperation) -> Vec<String> {
        use crate::forge::dryrun::OperationKind;

        match operation.op {
            OperationKind::CreateThread => vec![
                "gitlab".to_string(),
                crate::model::thread_store::root_comment_mapping_key("gitlab"),
            ],
            OperationKind::Reply { .. } => Vec::new(),
            OperationKind::Resolve | OperationKind::Reopen | OperationKind::Dismiss => {
                vec!["gitlab".to_string()]
            }
            OperationKind::SubmitReview { .. } => Vec::new(),
        }
    }

    /// Like `start_submit`, but when `skip_confirm` is `true` the flow
    /// bypasses `SubmitConfirm`. The action-picker path uses this because
    /// picking IS the confirmation; the resolver (if any unmappable
    /// comments) still runs first, then dispatches the network call
    /// directly. `:submit <event>` callers should pass `false`.
    pub fn start_submit_with(
        &mut self,
        event: crate::forge::submit::SubmitEvent,
        skip_confirm: bool,
    ) {
        use crate::forge::submit::{
            CommentAnchor, InlineComment, ResolverAction, UnmappableItem, map_comment,
        };

        let DiffSource::PullRequest(pr) = &self.diff_source else {
            self.set_warning(":submit only applies in PR mode");
            return;
        };
        if pr.is_read_only() {
            let reason = pr.read_only_reason().unwrap_or("read only");
            self.set_warning(format!("Cannot submit: PR is {reason}"));
            return;
        }
        // When the inline commit selector shows a strict subset, comments
        // anchor to the displayed (subset) diff, so `commit_id` must be the
        // SHA the diff was computed against — otherwise GitHub rejects with
        // 422 because the line/position isn't present in the diff against
        // the cumulative PR head. `pr_commits` is stored newest-first, so
        // the head of a (start_idx..=end_idx) range is `pr_commits[start_idx]`.
        let commit_id = match self.commit_selection_range {
            Some((start_idx, end_idx))
                if !self.pr_commits.is_empty()
                    && start_idx <= end_idx
                    && end_idx < self.pr_commits.len()
                    && !(start_idx == 0 && end_idx + 1 == self.pr_commits.len()) =>
            {
                self.pr_commits[start_idx].oid.clone()
            }
            _ => pr.key.head_sha.clone(),
        };

        // Source of truth for the diff: when the inline commit selector is
        // showing a strict subset, `range_diff_files` carries the merged
        // subset diff; otherwise `diff_files` is canonical.
        let files: Vec<&DiffFile> = match self.range_diff_files.as_ref() {
            Some(range) => range.iter().collect(),
            None => self.diff_files.iter().collect(),
        };

        let mut mappable: Vec<InlineComment> = Vec::new();
        let mut unmappable: Vec<UnmappableItem> = Vec::new();
        let mut total_local_drafts = 0_usize;

        // Walk file-level and line comments in display order. Review-level
        // comments (session.review_comments) are NOT inline-mapped; they
        // appear in the body via `build_review_body`.
        for file in &files {
            let Some(review) = self.session.files.get(file.display_path()) else {
                continue;
            };
            for comment in &review.file_comments {
                if comment.is_locked() || !self.comment_visible(comment) {
                    continue;
                }
                total_local_drafts += 1;
                bucket_mapping(
                    map_comment(comment, CommentAnchor::FileLevel, file, &self.forge_config),
                    &mut mappable,
                    &mut unmappable,
                );
            }
            let mut keys: Vec<&u32> = review.line_comments.keys().collect();
            keys.sort();
            for key in keys {
                for comment in &review.line_comments[key] {
                    if comment.is_locked() || !self.comment_visible(comment) {
                        continue;
                    }
                    total_local_drafts += 1;
                    let anchor = if comment.line_range.is_some() {
                        CommentAnchor::Range
                    } else {
                        CommentAnchor::Line {
                            line: *key,
                            side: comment.side.unwrap_or_default(),
                        }
                    };
                    bucket_mapping(
                        map_comment(comment, anchor, file, &self.forge_config),
                        &mut mappable,
                        &mut unmappable,
                    );
                }
            }
        }

        let durable_publication = if pr.key.repository.kind
            == crate::forge::traits::ForgeKind::GitLab
            && event == crate::forge::submit::SubmitEvent::Comment
            && self.session.review_comments.is_empty()
            && unmappable.is_empty()
        {
            match Self::build_gitlab_publication(&self.session, &mappable) {
                Ok((session, plan, fallback)) if fallback.is_empty() => {
                    Some((session, plan, fallback))
                }
                Ok(_) => None,
                Err(error) => {
                    self.set_error(format!("Cannot prepare GitLab publication: {error}"));
                    return;
                }
            }
        } else {
            None
        };

        // Approve is the one event that's meaningful with no comments — a
        // bare "LGTM" approval. Durable-only GitLab activity is also valid.
        let bare_allowed = matches!(event, crate::forge::submit::SubmitEvent::Approve);
        let has_durable_activity =
            durable_publication
                .as_ref()
                .is_some_and(|(_, plan, fallback)| {
                    !fallback.is_empty() || plan.operations.iter().any(|op| op.thread_id.is_some())
                });
        if !bare_allowed
            && total_local_drafts == 0
            && self.session.review_comments.is_empty()
            && !has_durable_activity
        {
            self.set_warning("Nothing to submit — no local-draft comments");
            return;
        }

        let resolver_choices = vec![ResolverAction::default(); unmappable.len()];
        let has_unmappable = !unmappable.is_empty();
        self.submit_durable_session = durable_publication
            .as_ref()
            .map(|(session, _, _)| session.clone());
        self.submit_durable_plan = durable_publication
            .as_ref()
            .map(|(_, plan, _)| plan.clone());
        self.submit_legacy_fallback_ids = durable_publication
            .map(|(_, _, fallback)| fallback)
            .unwrap_or_default();
        self.submit_state = Some(SubmitState {
            event,
            mappable,
            unmappable,
            resolver_choices,
            resolver_cursor: 0,
            commit_id,
            skip_confirm,
        });

        if has_unmappable {
            self.input_mode = InputMode::SubmitResolver;
        } else if skip_confirm {
            self.input_mode = InputMode::Normal;
            self.confirm_submit();
        } else {
            self.input_mode = InputMode::SubmitConfirm;
        }
    }

    /// Open the bare-`:submit` action picker. The user picks
    /// Comment/Approve/Request changes/Draft (or cancels); the picked event
    /// then runs through preflight with `skip_confirm = true` so no extra
    /// confirmation modal follows.
    pub fn start_submit_action_picker(&mut self) {
        if !matches!(self.diff_source, DiffSource::PullRequest(_)) {
            self.set_warning(":submit only applies in PR mode");
            return;
        }
        self.submit_picker_cursor = 0;
        self.input_mode = InputMode::SubmitActionPicker;
    }

    /// Move the action-picker cursor down by one row, wrapping at the end.
    pub fn submit_picker_cursor_down(&mut self) {
        let total = SUBMIT_PICKER_EVENTS.len();
        if total > 0 {
            self.submit_picker_cursor = (self.submit_picker_cursor + 1) % total;
        }
    }

    /// Move the action-picker cursor up by one row, wrapping at the start.
    pub fn submit_picker_cursor_up(&mut self) {
        let total = SUBMIT_PICKER_EVENTS.len();
        if total > 0 {
            self.submit_picker_cursor = (self.submit_picker_cursor + total - 1) % total;
        }
    }

    /// Confirm the action picker selection: dispatch into preflight with the
    /// chosen event and `skip_confirm = true`.
    pub fn submit_picker_confirm(&mut self) {
        let Some(event) = SUBMIT_PICKER_EVENTS
            .get(self.submit_picker_cursor)
            .map(|(_, ev)| *ev)
        else {
            self.cancel_submit_action_picker();
            return;
        };
        self.input_mode = InputMode::Normal;
        self.start_submit_with(event, true);
    }

    /// Cancel the action picker without entering preflight.
    pub fn cancel_submit_action_picker(&mut self) {
        self.input_mode = InputMode::Normal;
        self.submit_picker_cursor = 0;
    }

    pub fn cancel_submit(&mut self) {
        self.submit_state = None;
        self.submit_durable_session = None;
        self.submit_durable_plan = None;
        self.submit_legacy_fallback_ids.clear();
        self.input_mode = InputMode::Normal;
    }

    /// Move the resolver cursor down by one row, clamped to the last row.
    pub fn submit_resolver_cursor_down(&mut self) {
        if let Some(state) = self.submit_state.as_mut()
            && state.resolver_cursor + 1 < state.unmappable.len()
        {
            state.resolver_cursor += 1;
        }
    }

    pub fn submit_resolver_cursor_up(&mut self) {
        if let Some(state) = self.submit_state.as_mut()
            && state.resolver_cursor > 0
        {
            state.resolver_cursor -= 1;
        }
    }

    pub fn submit_resolver_toggle(&mut self) {
        use crate::forge::submit::ResolverAction;
        if let Some(state) = self.submit_state.as_mut()
            && let Some(choice) = state.resolver_choices.get_mut(state.resolver_cursor)
        {
            *choice = match choice {
                ResolverAction::MoveToSummary => ResolverAction::Omit,
                ResolverAction::Omit => ResolverAction::MoveToSummary,
            };
        }
    }

    /// Advance from the resolver. When `skip_confirm` is set (action-picker
    /// path), dispatch the network call directly; otherwise route to
    /// `SubmitConfirm` for the final confirmation modal.
    pub fn submit_resolver_advance(&mut self) {
        let Some(state) = self.submit_state.as_ref() else {
            return;
        };
        if state.skip_confirm {
            self.input_mode = InputMode::Normal;
            self.confirm_submit();
        } else {
            self.input_mode = InputMode::SubmitConfirm;
        }
    }

    /// True iff the original review head and the latest known PR head
    /// disagree. PR 5 cannot trigger this (the open-time head equals
    /// `current_pr_head`), but the field is exposed so the renderer can
    /// fold the warning in once PR 6 refreshes the remote head.
    pub fn submit_head_is_stale(&self) -> bool {
        let Some(state) = self.submit_state.as_ref() else {
            return false;
        };
        match self.current_pr_head.as_deref() {
            Some(latest) => latest != state.commit_id,
            None => false,
        }
    }

    /// Confirm submit — PR 6 dispatches the async `gh api .../reviews` call.
    /// Builds the body + payload on the main thread, saves the session, then
    /// hands off to `spawn_pr_submit`. The modal disappears immediately; a
    /// status-bar spinner takes over until the result lands in
    /// `poll_pr_submit_events`.
    pub fn confirm_submit(&mut self) {
        if let Err(e) = self.spawn_pr_submit() {
            self.set_error(format!("Submit failed: {e}"));
            self.submit_state = None;
            self.input_mode = InputMode::Normal;
        }
    }

    /// Kick off the create-review call asynchronously. Pre-submit-saves the
    /// session, builds the JSON payload on the main thread, then runs the
    /// network round-trip on a background thread. The result is applied
    /// later in `poll_pr_submit_events`.
    pub fn spawn_pr_submit(&mut self) -> Result<()> {
        use crate::forge::submit::{MovedToSummaryItem, ResolverAction, build_review_body};
        use crate::forge::traits::{CreateReviewRequest, PullRequestTarget};

        // Snapshot identity from the PR diff source first so the borrow on
        // `submit_state` below doesn't conflict.
        let DiffSource::PullRequest(pr) = self.diff_source.clone() else {
            return Err(TuicrError::UnsupportedOperation(
                "Not in PR mode".to_string(),
            ));
        };
        if self.pr_submit_state.is_some() {
            return Ok(()); // already in flight; ignore
        }

        let Some(state) = self.submit_state.take() else {
            return Ok(());
        };

        let summary_items: Vec<MovedToSummaryItem> = state
            .unmappable
            .iter()
            .zip(state.resolver_choices.iter())
            .filter_map(|(item, action)| {
                if *action == ResolverAction::MoveToSummary {
                    Some(MovedToSummaryItem {
                        comment: item.comment.clone(),
                        file: item.file.clone(),
                    })
                } else {
                    None
                }
            })
            .collect();
        let summary_comment_ids: Vec<String> =
            summary_items.iter().map(|i| i.comment.id.clone()).collect();
        let review_comment_ids: Vec<String> = self
            .session
            .review_comments
            .iter()
            .map(|c| c.id.clone())
            .collect();
        let body = build_review_body(
            &self.session.review_comments,
            &summary_items,
            &self.forge_config,
        );

        // Save the session BEFORE the network call — keeps the user's
        // local-draft work durable if anything goes sideways below.
        if let Err(error) = self.save_current_session_merging_external()
            && self.submit_durable_plan.is_some()
        {
            self.submit_durable_session = None;
            self.submit_durable_plan = None;
            self.submit_legacy_fallback_ids.clear();
            return Err(TuicrError::Forge(format!(
                "cannot start durable publication before saving the session: {error}"
            )));
        }
        let publication_identity = self.session.clone();

        let in_flight = SubmitInFlightState {
            event: state.event,
            mappable: state.mappable.clone(),
            summary_comment_ids,
            review_comment_ids,
            legacy_fallback_comment_ids: self.submit_legacy_fallback_ids.clone(),
            moved_to_summary_count: summary_items.len(),
            head_sha_snapshot: state.commit_id.clone(),
            pr_head_snapshot: pr.key.head_sha.clone(),
            repository: pr.key.repository.clone(),
            pr_number: pr.key.number,
            started_at: Instant::now(),
        };
        self.pr_submit_state = Some(in_flight.clone());
        self.input_mode = InputMode::Normal;

        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|backend| backend.local_checkout_path());

        let (tx, rx) = std::sync::mpsc::channel();
        self.pr_submit_rx = Some(rx);

        let repository = in_flight.repository.clone();
        let pr_number = in_flight.pr_number;
        let head_sha = in_flight.head_sha_snapshot.clone();
        let pr_head_sha = in_flight.pr_head_snapshot.clone();
        let event = in_flight.event;
        let mappable = in_flight.mappable.clone();
        let commit_id = state.commit_id.clone();
        let durable_session = self.submit_durable_session.take();
        let durable_plan = self.submit_durable_plan.take();
        let legacy_fallback_ids: std::collections::HashSet<&str> = self
            .submit_legacy_fallback_ids
            .iter()
            .map(String::as_str)
            .collect();
        let legacy_fallback_comments: Vec<_> = state
            .mappable
            .iter()
            .filter(|comment| legacy_fallback_ids.contains(comment.comment_id.as_str()))
            .cloned()
            .collect();
        self.submit_legacy_fallback_ids.clear();

        if let (Some(mut durable_session), Some(durable_plan)) = (durable_session, durable_plan) {
            std::thread::spawn(move || {
                let backend = create_forge_backend(&repository, local_checkout);
                let target = PullRequestTarget::with_repository(
                    repository.clone(),
                    pr_number,
                    pr_number.to_string(),
                );
                let result = backend
                    .get_pull_request(target)
                    .map_err(|error| error.to_string())
                    .and_then(|details| {
                        if details.repository != repository
                            || details.number != pr_number
                            || details.head_sha != pr_head_sha
                        {
                            return Err(
                                "merge request changed after confirmation; reload before publishing"
                                    .to_string(),
                            );
                        }
                        let report =
                            crate::forge::publish::execute_plan_with_checkpoint_and_comments(
                                backend.as_ref(),
                                &details,
                                &mut durable_session,
                                &durable_plan,
                                &body,
                                &legacy_fallback_comments,
                                |published, operation| {
                                    Self::checkpoint_publication_mapping(
                                        &publication_identity,
                                        published,
                                        operation,
                                    )
                                },
                            );
                        let recovery = Self::recover_publication_checkpoints(
                            &publication_identity,
                            &durable_session,
                            &report,
                        )
                        .map_err(|error| error.to_string());
                        Ok(DurableSubmitOutcome {
                            session: durable_session,
                            report,
                            recovery,
                        })
                    });
                let _ = tx.send(PrSubmitEvent::DurableDone {
                    repository,
                    pr_number,
                    head_sha,
                    result: Box::new(result),
                });
            });
            return Ok(());
        }

        std::thread::spawn(move || {
            let backend = create_forge_backend(&repository, local_checkout);
            // Need PR details for repo/owner routing; refetch lightly via
            // the same target the user opened with.
            let target = PullRequestTarget::with_repository(
                repository.clone(),
                pr_number,
                pr_number.to_string(),
            );
            let result = match backend.get_pull_request(target) {
                Ok(details) => backend
                    .create_review(
                        &details,
                        CreateReviewRequest {
                            event,
                            commit_id: &commit_id,
                            body: &body,
                            comments: &mappable,
                        },
                    )
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(PrSubmitEvent::Done {
                repository,
                pr_number,
                head_sha,
                result,
            });
        });
        Ok(())
    }

    /// Pump a pending create-review result. Applies lifecycle writes + the
    /// success message, or surfaces a sticky error.
    pub fn poll_pr_submit_events(&mut self) {
        let Some(rx) = self.pr_submit_rx.as_ref() else {
            return;
        };
        let event = match rx.try_recv() {
            Ok(e) => e,
            Err(_) => return,
        };
        self.pr_submit_rx = None;
        let Some(in_flight) = self.pr_submit_state.take() else {
            return;
        };

        match event {
            PrSubmitEvent::Done {
                repository,
                pr_number,
                head_sha,
                result,
            } => {
                if self.submit_result_is_stale(&in_flight, &repository, pr_number, &head_sha) {
                    self.set_message("Discarded stale submit result (PR was reloaded)".to_string());
                    return;
                }
                self.finish_pr_submit(in_flight, result);
            }
            PrSubmitEvent::DurableDone {
                repository,
                pr_number,
                head_sha,
                result,
            } => {
                if self.submit_result_is_stale(&in_flight, &repository, pr_number, &head_sha) {
                    match *result {
                        Ok(DurableSubmitOutcome {
                            recovery: Ok(()), ..
                        }) => self.set_message(
                            "GitLab publication finished for the previous MR revision; mappings were checkpointed"
                                .to_string(),
                        ),
                        Ok(DurableSubmitOutcome {
                            recovery: Err(error),
                            ..
                        })
                        | Err(error) => self.set_error(format!(
                            "GitLab publication finished for the previous MR revision, but checkpoint recovery failed: {error}"
                        )),
                    }
                    return;
                }
                match *result {
                    Ok(outcome) => self.finish_durable_gitlab_submit(
                        in_flight,
                        outcome.session,
                        outcome.report,
                    ),
                    Err(error) => self.set_error(format!("Submit failed: {error}")),
                }
            }
        }
    }

    fn finish_durable_gitlab_submit(
        &mut self,
        in_flight: SubmitInFlightState,
        published: crate::model::review::ReviewSession,
        report: crate::forge::publish::PublishReport,
    ) {
        use crate::forge::dryrun::OperationKind;
        use crate::forge::publish::ExecutedOperation;

        let mut completed_comment_ids = Vec::new();
        let mut review_completed = false;
        let mut completed_count = 0;
        let mut skipped_count = 0;
        for result in &report.results {
            match result {
                ExecutedOperation::Completed(operation) => {
                    completed_count += 1;
                    if let Some(thread_id) = operation.thread_id.as_deref()
                        && let Some(published_thread) = published
                            .threads
                            .iter()
                            .find(|thread| thread.id().as_str() == thread_id)
                        && let Some(current) = self
                            .session
                            .threads
                            .iter_mut()
                            .find(|thread| thread.id().as_str() == thread_id)
                    {
                        for key in Self::publication_mapping_keys(operation) {
                            if let Some(mapping) = published_thread.provider_mappings.get(&key) {
                                current.provider_mappings.insert(key, mapping.clone());
                            }
                        }
                    }
                    match &operation.op {
                        OperationKind::CreateThread => {
                            if let Some(thread_id) = operation.thread_id.as_deref()
                                && let Some(thread) = published
                                    .threads
                                    .iter()
                                    .find(|thread| thread.id().as_str() == thread_id)
                                && let Some(root) = thread.thread.root()
                            {
                                completed_comment_ids.push(root.id().as_str().to_string());
                            }
                        }
                        OperationKind::Reply { comment_id } => {
                            if let Some(thread_id) = operation.thread_id.as_deref()
                                && let Some(published_thread) = published
                                    .threads
                                    .iter()
                                    .find(|thread| thread.id().as_str() == thread_id)
                                && let Some(remote_id) =
                                    published_thread.published_reply_id("gitlab", comment_id)
                                && let Some(current) = self
                                    .session
                                    .threads
                                    .iter_mut()
                                    .find(|thread| thread.id().as_str() == thread_id)
                            {
                                current.record_published_reply(
                                    "gitlab",
                                    comment_id.clone(),
                                    remote_id.to_string(),
                                );
                            }
                            completed_comment_ids.push(comment_id.clone());
                        }
                        OperationKind::SubmitReview { .. } => review_completed = true,
                        OperationKind::Resolve | OperationKind::Reopen | OperationKind::Dismiss => {
                        }
                    }
                }
                ExecutedOperation::Skipped(_) => skipped_count += 1,
            }
        }

        self.mark_legacy_comments_submitted(&completed_comment_ids, None);
        if review_completed {
            let review_id = report
                .review_response
                .as_ref()
                .map(|response| response.id.to_string());
            let body_ids: Vec<String> = in_flight
                .summary_comment_ids
                .iter()
                .chain(in_flight.review_comment_ids.iter())
                .chain(in_flight.legacy_fallback_comment_ids.iter())
                .cloned()
                .collect();
            self.mark_legacy_comments_submitted(&body_ids, review_id.as_deref());
            self.mark_pr_commits_reviewed_through(&in_flight.head_sha_snapshot);
        }

        let save_result = self.save_current_session_merging_external();
        self.refetch_pr_threads();

        if let Some((operation, error)) = report.failed {
            if let Err(save_error) = save_result {
                self.set_error(format!(
                    "GitLab publication stopped after {completed_count} operation(s) at {:?}: {error}; mapping recovery also failed: {save_error}",
                    operation.op
                ));
            } else {
                self.set_error(format!(
                    "GitLab publication stopped after {completed_count} operation(s) at {:?}: {error}; retry will resume",
                    operation.op
                ));
            }
        } else if let Err(error) = save_result {
            self.set_error(format!(
                "GitLab publication completed, but saving its mappings failed: {error}"
            ));
        } else {
            if in_flight.event == crate::forge::submit::SubmitEvent::Comment {
                self.mark_pr_commits_reviewed_through(&in_flight.head_sha_snapshot);
            }
            self.set_message(format!(
                "Published GitLab review: {completed_count} completed, {skipped_count} skipped"
            ));
        }
    }

    fn mark_legacy_comments_submitted(&mut self, ids: &[String], remote_id: Option<&str>) {
        use crate::model::comment::CommentLifecycleState;

        let ids: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
        if ids.is_empty() {
            return;
        }
        let mark = |comment: &mut crate::model::comment::Comment| {
            if ids.contains(comment.id.as_str()) {
                comment.lifecycle_state = CommentLifecycleState::Submitted;
                if let Some(remote_id) = remote_id {
                    comment.remote_review_id = Some(remote_id.to_string());
                }
            }
        };
        for comment in &mut self.session.review_comments {
            mark(comment);
        }
        for review in self.session.files.values_mut() {
            for comment in &mut review.file_comments {
                mark(comment);
            }
            for comments in review.line_comments.values_mut() {
                for comment in comments {
                    mark(comment);
                }
            }
        }
        self.rebuild_annotations();
    }

    /// Human-readable name of the forge backing the current PR/MR review.
    /// Used to keep submit messaging accurate across GitHub and GitLab.
    pub fn forge_display_name(&self) -> &'static str {
        match &self.diff_source {
            DiffSource::PullRequest(pr) => match pr.key.repository.kind {
                crate::forge::traits::ForgeKind::GitHub => "GitHub",
                crate::forge::traits::ForgeKind::GitLab => "GitLab",
                crate::forge::traits::ForgeKind::AzureDevOps => "Azure DevOps",
                crate::forge::traits::ForgeKind::Gitea => "Gitea",
                crate::forge::traits::ForgeKind::Forgejo => "Forgejo",
            },
            _ => "forge",
        }
    }

    /// Apply the create-review result on the main thread. On success: flip
    /// each included `Comment` to `Submitted` (or `PushedDraft` for the
    /// draft event), stamp `remote_review_id`, save the session again, and
    /// publish a success message. On failure: keep everything as
    /// `LocalDraft` and set a sticky error.
    pub fn finish_pr_submit(
        &mut self,
        in_flight: SubmitInFlightState,
        result: std::result::Result<crate::forge::traits::GhCreateReviewResponse, String>,
    ) {
        use crate::forge::submit::SubmitEvent;

        let response = match result {
            Ok(r) => r,
            Err(e) => {
                self.set_error(format!("Submit failed: {e}"));
                return;
            }
        };

        self.apply_submit_success(&in_flight, &response);

        // Post-submit save — captures the lifecycle transitions.
        let _ = self.save_current_session_merging_external();

        let inline_count = in_flight.mappable.len();
        let summary_count = in_flight.moved_to_summary_count;
        let forge_name = self.forge_display_name();
        let message = match in_flight.event {
            SubmitEvent::Draft => {
                let pr_url = match &self.diff_source {
                    DiffSource::PullRequest(pr) => pr.url.clone(),
                    _ => String::new(),
                };
                if pr_url.is_empty() {
                    format!(
                        "Pushed pending {forge_name} review #{}: {} inline, {} moved to summary",
                        response.id, inline_count, summary_count,
                    )
                } else {
                    format!(
                        "Pushed pending {forge_name} review #{}: {} inline, {} moved to summary — Finish it in {forge_name}: {}",
                        response.id, inline_count, summary_count, pr_url,
                    )
                }
            }
            _ => format!(
                "Submitted {forge_name} review #{}: {} inline, {} moved to summary",
                response.id, inline_count, summary_count,
            ),
        };
        if in_flight.event != SubmitEvent::Draft {
            self.mark_pr_commits_reviewed_through(&in_flight.head_sha_snapshot);
        }
        self.set_message(message);

        // Refetch remote threads so the just-submitted comments appear immediately.
        self.refetch_pr_threads();
    }

    /// Flip every comment that was sent — inline, summary-bound, and review-
    /// level — from `LocalDraft` to `Submitted` (or `PushedDraft` for
    /// `:submit draft`) and stamp `remote_review_id`. The comments stay in
    /// the session so the user keeps seeing their work; they're pruned by
    /// `prune_locked_comments` when remote threads are next fetched.
    pub fn apply_submit_success(
        &mut self,
        in_flight: &SubmitInFlightState,
        response: &crate::forge::traits::GhCreateReviewResponse,
    ) {
        use crate::forge::submit::SubmitEvent;
        use crate::model::comment::CommentLifecycleState;

        let new_state = match in_flight.event {
            SubmitEvent::Draft => CommentLifecycleState::PushedDraft,
            _ => CommentLifecycleState::Submitted,
        };
        let review_id = response.id.to_string();

        let target_ids: std::collections::HashSet<&str> = in_flight
            .mappable
            .iter()
            .map(|c| c.comment_id.as_str())
            .chain(in_flight.summary_comment_ids.iter().map(String::as_str))
            .chain(in_flight.review_comment_ids.iter().map(String::as_str))
            .collect();
        if target_ids.is_empty() {
            return;
        }

        for comment in self.session.review_comments.iter_mut() {
            if target_ids.contains(comment.id.as_str()) {
                comment.lifecycle_state = new_state;
                comment.remote_review_id = Some(review_id.clone());
            }
        }
        for review in self.session.files.values_mut() {
            for comment in review.file_comments.iter_mut() {
                if target_ids.contains(comment.id.as_str()) {
                    comment.lifecycle_state = new_state;
                    comment.remote_review_id = Some(review_id.clone());
                }
            }
            for comments in review.line_comments.values_mut() {
                for comment in comments.iter_mut() {
                    if target_ids.contains(comment.id.as_str()) {
                        comment.lifecycle_state = new_state;
                        comment.remote_review_id = Some(review_id.clone());
                    }
                }
            }
        }
        self.rebuild_annotations();
    }

    /// Drop locked (`Submitted`/`PushedDraft`) comments from the session.
    /// Called after a successful `forge_review_threads` fetch: anything that
    /// was published to the forge is now represented by the fresh remote
    /// threads, so keeping the locals would double-render every line.
    pub fn prune_locked_comments(&mut self) {
        self.session.review_comments.retain(|c| !c.is_locked());
        for review in self.session.files.values_mut() {
            review.file_comments.retain(|c| !c.is_locked());
            for comments in review.line_comments.values_mut() {
                comments.retain(|c| !c.is_locked());
            }
            review.line_comments.retain(|_, v| !v.is_empty());
        }
    }
}
