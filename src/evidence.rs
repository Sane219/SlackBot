//! Turning collected Activity into the text a model writes from.
//!
//! ADR-0005: Evidence is rendered text, not a structured document, and every count in it
//! describes what **survived the trim** rather than what was fetched. A Draft built from
//! 15 messages reads as a 15-message day.

use chrono::{DateTime, Utc};
use std::fmt::Write as _;

use crate::domain::Counts;
use crate::github::GithubActivity;
use crate::slack::SlackMessage;

/// One thing the user did, from either integration.
#[derive(Debug, Clone)]
pub enum Activity {
    Slack {
        channel: String,
        message: SlackMessage,
    },
    Github(GithubActivity),
}

/// Which sources produced evidence. A missing source must be named, never left as a
/// hole the model fills in (ADR-0007, partial Fires).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceStatus {
    pub slack_ok: bool,
    pub github_ok: bool,
}

impl SourceStatus {
    pub fn both() -> Self {
        Self {
            slack_ok: true,
            github_ok: true,
        }
    }
}

/// What the renderer decided to include, and what it left out.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub text: String,
    pub counts: Counts,
    /// Nothing at all was collected. The Draft says so rather than inventing content.
    pub no_signal: bool,
    /// One source failed, and is named in the Evidence.
    pub partial: bool,
}

/// Roughly four characters per token. Good enough for a budget that exists to stop a
/// window costing a fortune, not to be exact.
fn approx_tokens(text: &str) -> usize {
    text.len() / 4
}

/// Collapse Slack's inline markup into plain text.
///
/// Unescaped `&<>` mangles mrkdwn if it reaches the model raw, and `<@U123>` is noise to a
/// model that cannot resolve it to a name anyway.
fn sanitize_slack(input: &str) -> String {
    let mut out = input.to_string();

    // <@U123|name> and <@U123> -> name, or @user when there is no label.
    out = replace_mentions(&out);
    // <https://url|label> and <https://url> -> label or url.
    out = replace_links(&out);
    // Special entities.
    for (entity, replacement) in [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("<!here>", "@here"),
        ("<!channel>", "@channel"),
        ("<!everyone>", "@everyone"),
    ] {
        out = out.replace(entity, replacement);
    }

    out
}

/// Walk `<@…>` and `<!…>` spans, keeping the label where one exists.
///
/// An unmatched `<` copies the remainder verbatim rather than breaking: a message
/// containing "a < b" would otherwise lose everything after it.
fn replace_mentions(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find('<') {
        let Some(end_offset) = rest[start..].find('>') else {
            break;
        };
        let end = start + end_offset;
        let inner = &rest[start + 1..end];

        out.push_str(&rest[..start]);

        // A mention is `@`/`!` followed by an id, optionally `|label`.
        if inner.starts_with('@') || inner.starts_with('!') {
            match inner.split_once('|') {
                Some((_, label)) if !label.is_empty() => {
                    let _ = write!(out, "@{label}");
                }
                _ => {
                    // No label: keep it recognisable without pretending to resolve it.
                    let _ = write!(out, "@user");
                }
            }
        } else {
            // Not a mention. Copy it through unchanged and let the link pass handle it.
            out.push_str(&rest[start..=end]);
        }

        rest = &rest[end + 1..];
    }

    out.push_str(rest);
    out
}

/// Walk `<url|label>` and `<url>` spans, preserving everything outside them.
fn replace_links(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find('<') {
        let Some(end_offset) = rest[start..].find('>') else {
            break;
        };
        let end = start + end_offset;
        let inner = &rest[start + 1..end];

        out.push_str(&rest[..start]);

        // Only treat it as a link if it actually looks like one. `inner == "<"` is the
        // empty span that `a < b` produces, and it must survive.
        if inner.starts_with("http://")
            || inner.starts_with("https://")
            || inner.starts_with("mailto:")
        {
            match inner.split_once('|') {
                Some((_, label)) if !label.is_empty() => out.push_str(label),
                _ => out.push_str(inner),
            }
        } else {
            out.push_str(&rest[start..=end]);
        }

        rest = &rest[end + 1..];
    }

    out.push_str(rest);
    out
}

fn first_line(body: &str) -> String {
    let line = body.lines().next().unwrap_or("").trim();
    // Comment bodies are often quoted or padded; keep the first meaningful line.
    if line.is_empty() {
        body.lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("")
            .to_string()
    } else {
        line.to_string()
    }
}

fn clip(body: &str, max: usize) -> String {
    let single = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if single.chars().count() <= max {
        return single;
    }
    let truncated: String = single.chars().take(max).collect();
    format!("{truncated}…")
}

/// Render Activity into Evidence.
///
/// Activity is trimmed **oldest-first**: the most recent work is kept, because a status
/// post written at 18:30 is about what happened recently. `budget_tokens` bounds the
/// whole block; the coverage line always states what was dropped.
pub fn render(
    activity: &[Activity],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    status: SourceStatus,
    budget_tokens: usize,
) -> Rendered {
    let slack_fetched = activity
        .iter()
        .filter(|a| matches!(a, Activity::Slack { .. }))
        .count() as u32;
    let github_fetched = activity
        .iter()
        .filter(|a| matches!(a, Activity::Github(_)))
        .count() as u32;

    let slack_channels = activity
        .iter()
        .filter_map(|a| match a {
            Activity::Slack { channel, .. } => Some(channel.clone()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
        .len() as u32;
    let github_repos = activity
        .iter()
        .filter_map(|a| match a {
            Activity::Github(g) => Some(g.repo.clone()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
        .len() as u32;

    let mut counts = Counts {
        slack_channels,
        github_repos,
        slack_fetched,
        github_fetched,
        ..Counts::default()
    };

    // Newest first, so trimming the tail drops the oldest work.
    let mut ordered: Vec<&Activity> = activity.iter().collect();
    ordered.sort_by_key(|a| std::cmp::Reverse(activity_time(a)));

    let mut lines: Vec<String> = Vec::new();
    let mut kept_slack = 0u32;
    let mut kept_github = 0u32;
    let mut budget_used = 0usize;

    for item in &ordered {
        let line = match item {
            Activity::Slack { channel, message } => {
                let text = sanitize_slack(message.text.as_deref().unwrap_or(""));
                let text = clip(&text, 240);
                if text.is_empty() {
                    continue;
                }
                let when = activity_time(item).format("%H:%M");
                format!("- {when} #{channel} {text}")
            }
            Activity::Github(g) => {
                let code = g.kind.code();
                let body = clip(&first_line(&g.body), 200);
                if body.is_empty() {
                    continue;
                }
                let when = g.created_at.format("%d %b %H:%M");
                format!(
                    "- {when} {code} {}/{} #{} {} {}",
                    g.repo, "", g.number, g.title, body
                )
                .trim_end()
                .to_string()
            }
        };

        let cost = approx_tokens(&line);
        if budget_used + cost > budget_tokens {
            // Out of budget. Stop rather than continue, so the remaining work is
            // reported as dropped rather than silently skipped in an order the reader
            // cannot predict.
            break;
        }

        budget_used += cost;
        match item {
            Activity::Slack { .. } => kept_slack += 1,
            Activity::Github(_) => kept_github += 1,
        }
        lines.push(line);
    }

    counts.slack_messages = kept_slack;
    counts.github_comments = kept_github;

    let no_signal = counts.is_empty();
    let partial = status.slack_ok != status.github_ok;

    let mut text = String::new();

    let _ = writeln!(
        text,
        "## Activity  {} → {}",
        from.format("%d %b %H:%M"),
        to.format("%d %b %H:%M")
    );

    if partial {
        // Named, so the model does not write a confident Slack paragraph from nothing.
        let missing = if !status.slack_ok { "Slack" } else { "GitHub" };
        let _ = writeln!(
            text,
            "## Unavailable: {missing} could not be read for this window. Do not write about {missing} activity."
        );
    }

    if no_signal {
        let _ = writeln!(text, "## No activity recorded in this window.");
    } else {
        text.push('\n');
        text.push_str(&lines.join("\n"));
    }

    // The coverage line. ADR-0005: every number here describes what survived.
    let _ = write!(
        text,
        "\n## Coverage: {} of {} messages, {} of {} comments",
        kept_slack, slack_fetched, kept_github, github_fetched
    );
    if counts.is_truncated() {
        text.push_str(" (trimmed to the token budget; oldest first)");
    }

    Rendered {
        text,
        counts,
        no_signal,
        partial,
    }
}

fn activity_time(activity: &Activity) -> DateTime<Utc> {
    match activity {
        Activity::Slack { message, .. } => message.as_time().unwrap_or_else(Utc::now),
        Activity::Github(g) => g.created_at,
    }
}

/// The text a Draft carries when nothing was collected.
///
/// The memorable moment: a gap is drawn as a gap. Not a hedge, not a filler paragraph,
/// and explicitly not an invitation to infer activity.
pub const NO_SIGNAL_TEXT: &str = "NO SIGNAL";

/// Instructions appended to every draft prompt. Short on purpose: this is a status post,
/// not an essay, and the constraints below are the ones that make it look native.
pub fn draft_instructions() -> &'static str {
    "Write the status message only. No preamble, no explanation of what you did, no \
     surrounding quotes, no code fence.\n\
     Slack bold is a single asterisk: *bold*, never **bold**.\n\
     One point per line, each formatted as: • *Short title*: explanation\n\
     Reference issues and pull requests inline as #123 and as a Slack link.\n\
     Mention a person as <@U123> only when you are asking them for something.\n\
     If the activity does not support a point, write fewer points rather than padding.\n\
     If no activity was recorded, reply with exactly NO SIGNAL."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::ActivityKind;
    use crate::slack::SlackMessage;
    use chrono::TimeZone;

    fn msg(ts: i64, text: &str) -> SlackMessage {
        SlackMessage {
            ts: format!("{ts}.000100"),
            user: None,
            text: Some(text.into()),
            subtype: Some("message".into()),
            reply_count: None,
        }
    }

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 2, hour, 0, 0).unwrap()
    }

    fn window() -> (DateTime<Utc>, DateTime<Utc>) {
        (at(0), at(23))
    }

    #[test]
    fn empty_activity_renders_no_signal() {
        let (from, to) = window();
        let rendered = render(&[], from, to, SourceStatus::both(), 4000);

        assert!(rendered.no_signal);
        assert_eq!(rendered.counts.slack_messages, 0);
        assert!(rendered.text.contains("No activity recorded"));
    }

    #[test]
    fn the_no_signal_draft_text_is_exactly_the_gap_marker() {
        // Anything wordier becomes a sentence the model can fill.
        assert_eq!(NO_SIGNAL_TEXT, "NO SIGNAL");
    }

    #[test]
    fn mentions_are_reduced_to_readable_names() {
        let rendered = sanitize_slack("please review <@U0A9WPY4S1F|sanket> and <@UOTHER>");
        assert_eq!(rendered, "please review @sanket and @user");
    }

    #[test]
    fn links_keep_their_label() {
        assert_eq!(
            sanitize_slack("see <https://x.test/PR|PR #482>"),
            "see PR #482"
        );
        assert_eq!(
            sanitize_slack("see <https://x.test/PR>"),
            "see https://x.test/PR"
        );
    }

    #[test]
    fn html_entities_do_not_reach_the_model_raw() {
        let rendered = sanitize_slack("a &amp; b &lt;tag&gt; &lt;!here&gt; ping");
        assert_eq!(rendered, "a & b <tag> @here ping");
    }

    #[test]
    fn mentions_are_sanitized_in_rendered_evidence() {
        let (from, to) = window();
        let activity = vec![Activity::Slack {
            channel: "coot-ai".into(),
            message: msg(
                1_779_000_000,
                "cc <@U0A9WPY4S1F|nayab> on <https://x.test/PR|PR #482>",
            ),
        }];
        let rendered = render(&activity, from, to, SourceStatus::both(), 4000);
        assert!(rendered.text.contains("@nayab"));
        assert!(!rendered.text.contains("<@U0A9WPY4S1F"));
    }

    #[test]
    fn counts_describe_what_survived_not_what_was_fetched() {
        let (from, to) = window();
        let activity: Vec<Activity> = (0..40)
            .map(|i| Activity::Slack {
                channel: "coot-ai".into(),
                message: msg(1_779_000_000 + i * 60, &format!("message number {i}")),
            })
            .collect();

        // A budget small enough to bite.
        let rendered = render(&activity, from, to, SourceStatus::both(), 40);

        assert!(rendered.counts.slack_messages < 40);
        assert!(rendered.counts.is_truncated());
        // The coverage line must state the drop, or the model overstates the day.
        assert!(rendered
            .text
            .contains(&format!("of {} messages", rendered.counts.slack_fetched)));
        assert!(rendered.text.contains("trimmed"));
    }

    #[test]
    fn trimming_keeps_the_newest_activity() {
        let (from, to) = window();
        let activity: Vec<Activity> = (0..40)
            .map(|i| Activity::Slack {
                channel: "coot-ai".into(),
                message: msg(1_779_000_000 + i * 60, &format!("message number {i}")),
            })
            .collect();

        let rendered = render(&activity, from, to, SourceStatus::both(), 40);

        // Oldest first would have kept "message number 0".
        assert!(rendered.text.contains("message number 39"));
        assert!(!rendered.text.contains("message number 0"));
    }

    #[test]
    fn an_unavailable_source_is_named_and_forbidden() {
        let (from, to) = window();
        let activity = vec![Activity::Slack {
            channel: "coot-ai".into(),
            message: msg(1_779_000_000, "pushed the jitter fix"),
        }];

        let rendered = render(
            &activity,
            from,
            to,
            SourceStatus {
                slack_ok: true,
                github_ok: false,
            },
            4000,
        );

        assert!(rendered.partial);
        assert!(rendered.text.contains("GitHub could not be read"));
        assert!(rendered.text.contains("Do not write about GitHub"));
    }

    #[test]
    fn both_sources_fine_is_not_partial() {
        let (from, to) = window();
        let rendered = render(&[], from, to, SourceStatus::both(), 4000);
        assert!(!rendered.partial);
        assert!(!rendered.text.contains("Unavailable"));
    }

    #[test]
    fn instructions_state_the_slack_bold_rule() {
        // **bold** renders literally in Slack; this is the single highest-value constraint.
        assert!(draft_instructions().contains("*bold*, never **bold**"));
    }

    #[test]
    fn instructions_tell_the_model_to_say_nothing_rather_than_pad() {
        assert!(draft_instructions().contains("fewer points rather than padding"));
        assert!(draft_instructions().contains("NO SIGNAL"));
    }

    #[test]
    fn github_activity_is_labelled_by_kind() {
        let (from, to) = window();
        let activity = vec![Activity::Github(crate::github::GithubActivity {
            repo: "coot/ai".into(),
            number: 482,
            kind: ActivityKind::ReviewComment,
            title: "failover jitter".into(),
            body: "needs a rebase before merge\nmore detail".into(),
            created_at: at(10),
        })];

        let rendered = render(&activity, from, to, SourceStatus::both(), 4000);
        assert!(rendered.text.contains("PR coot/ai"));
        assert!(rendered.text.contains("needs a rebase"));
        // Only the first meaningful line survives.
        assert!(!rendered.text.contains("more detail"));
    }
}
