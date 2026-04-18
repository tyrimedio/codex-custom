use super::JobOutcome;
use super::JobResult;
use super::aggregate_stats;
use super::job::rollout_items_for_stage_one;
use super::job::serialize_filtered_rollout_response_items;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::TokenUsage;
use codex_protocol::protocol::TurnCompleteEvent;
use pretty_assertions::assert_eq;

#[test]
fn serializes_memory_rollout_with_agents_removed_but_environment_kept() {
    let mixed_contextual_message = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![
            ContentItem::InputText {
                text: "# AGENTS.md instructions for /tmp\n\n<INSTRUCTIONS>\nbody\n</INSTRUCTIONS>"
                    .to_string(),
            },
            ContentItem::InputText {
                text: "<environment_context>\n<cwd>/tmp</cwd>\n</environment_context>".to_string(),
            },
        ],
        end_turn: None,
        phase: None,
    };
    let skill_message = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "<skill>\n<name>demo</name>\n<path>skills/demo/SKILL.md</path>\nbody\n</skill>"
                .to_string(),
        }],
        end_turn: None,
        phase: None,
    };
    let subagent_message = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "<subagent_notification>{\"agent_id\":\"a\",\"status\":\"completed\"}</subagent_notification>"
                .to_string(),
        }],
        end_turn: None,
        phase: None,
    };

    let serialized = serialize_filtered_rollout_response_items(&[
        RolloutItem::ResponseItem(mixed_contextual_message),
        RolloutItem::ResponseItem(skill_message),
        RolloutItem::ResponseItem(subagent_message.clone()),
    ])
    .expect("serialize");
    let parsed: Vec<ResponseItem> = serde_json::from_str(&serialized).expect("parse");

    assert_eq!(
        parsed,
        vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "<environment_context>\n<cwd>/tmp</cwd>\n</environment_context>"
                        .to_string(),
                }],
                end_turn: None,
                phase: None,
            },
            subagent_message,
        ]
    );
}

#[test]
fn serializes_memory_rollout_redacts_secrets_before_prompt_upload() {
    let serialized = serialize_filtered_rollout_response_items(&[RolloutItem::ResponseItem(
        ResponseItem::FunctionCallOutput {
            call_id: "call_123".to_string(),
            output: FunctionCallOutputPayload {
                body: FunctionCallOutputBody::Text(
                    r#"{"token":"sk-abcdefghijklmnopqrstuvwxyz123456"}"#.to_string(),
                ),
                success: Some(true),
            },
        },
    )])
    .expect("serialize");

    assert!(!serialized.contains("sk-abcdefghijklmnopqrstuvwxyz123456"));
    assert!(serialized.contains("[REDACTED_SECRET]"));
}

#[test]
fn count_outcomes_sums_token_usage_across_all_jobs() {
    let counts = aggregate_stats(vec![
        JobResult {
            outcome: JobOutcome::SucceededWithOutput,
            token_usage: Some(TokenUsage {
                input_tokens: 10,
                cached_input_tokens: 2,
                output_tokens: 3,
                reasoning_output_tokens: 1,
                total_tokens: 13,
            }),
        },
        JobResult {
            outcome: JobOutcome::SucceededNoOutput,
            token_usage: Some(TokenUsage {
                input_tokens: 7,
                cached_input_tokens: 1,
                output_tokens: 2,
                reasoning_output_tokens: 0,
                total_tokens: 9,
            }),
        },
        JobResult {
            outcome: JobOutcome::Failed,
            token_usage: None,
        },
    ]);

    assert_eq!(counts.claimed, 3);
    assert_eq!(counts.succeeded_with_output, 1);
    assert_eq!(counts.succeeded_no_output, 1);
    assert_eq!(counts.failed, 1);
    assert_eq!(
        counts.total_token_usage,
        Some(TokenUsage {
            input_tokens: 17,
            cached_input_tokens: 3,
            output_tokens: 5,
            reasoning_output_tokens: 1,
            total_tokens: 22,
        })
    );
}

#[test]
fn count_outcomes_keeps_usage_empty_when_no_job_reports_it() {
    let counts = aggregate_stats(vec![
        JobResult {
            outcome: JobOutcome::SucceededWithOutput,
            token_usage: None,
        },
        JobResult {
            outcome: JobOutcome::Failed,
            token_usage: None,
        },
    ]);

    assert_eq!(counts.claimed, 2);
    assert_eq!(counts.total_token_usage, None);
}

#[test]
fn rollout_items_for_stage_one_stops_at_requested_turn_completion() {
    let before = RolloutItem::ResponseItem(ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "before".to_string(),
        }],
        end_turn: None,
        phase: None,
    });
    let target_complete = RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
        turn_id: "turn-1".to_string(),
        last_agent_message: Some("done".to_string()),
        completed_at: Some(1),
        duration_ms: Some(10),
    }));
    let after = RolloutItem::ResponseItem(ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: "after".to_string(),
        }],
        end_turn: None,
        phase: None,
    });
    let items = vec![before.clone(), target_complete.clone(), after];

    let truncated =
        rollout_items_for_stage_one(&items, Some("turn-1")).expect("truncate at target turn");

    assert_eq!(truncated.len(), 2);
    assert!(matches!(
        &truncated[0],
        RolloutItem::ResponseItem(ResponseItem::Message { role, .. }) if role == "user"
    ));
    assert!(matches!(
        &truncated[1],
        RolloutItem::EventMsg(EventMsg::TurnComplete(event)) if event.turn_id == "turn-1"
    ));
}

#[test]
fn rollout_items_for_stage_one_errors_when_turn_completion_is_missing() {
    let items = vec![RolloutItem::ResponseItem(ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "before".to_string(),
        }],
        end_turn: None,
        phase: None,
    })];

    let err =
        rollout_items_for_stage_one(&items, Some("missing-turn")).expect_err("missing cutoff");

    assert!(err.to_string().contains("missing-turn"));
}
