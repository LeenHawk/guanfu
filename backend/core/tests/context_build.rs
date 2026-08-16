//! ContextBuild fixture:ST 语义子集的编排结果。

use guanfu_core::assets::character::{CharacterV1, InjectionRole};
use guanfu_core::assets::persona::PersonaV1;
use guanfu_core::assets::preset::{
    ContextAssembly, OpenAiChatPresetV1, PromptInjection, PromptItem, PromptMarker,
    PromptOrderEntry, PromptOrderProfile,
};
use guanfu_core::assets::world_book::{WorldBookEntry, WorldBookV1};
use guanfu_core::context::{build_context, ContextSnapshot, HistoryTurn, TurnRole};
use guanfu_core::llm::ir::ModelId;

fn prompt(identifier: &str, content: &str, marker: Option<PromptMarker>) -> PromptItem {
    PromptItem {
        identifier: identifier.to_owned(),
        name: identifier.to_owned(),
        role: InjectionRole::System,
        content: content.to_owned(),
        marker,
        injection: PromptInjection::Relative,
        forbid_overrides: false,
        extra: Default::default(),
    }
}

fn entry(name: &str, content: &str, keys: &[&str]) -> WorldBookEntry {
    WorldBookEntry {
        name: name.to_owned(),
        content: content.to_owned(),
        keys: keys.iter().map(|key| (*key).to_owned()).collect(),
        enabled: true,
        ..Default::default()
    }
}

fn snapshot() -> ContextSnapshot {
    ContextSnapshot {
        character: Some(CharacterV1 {
            name: "Seraphina".into(),
            description: "Guardian of {{user}}'s glade".into(),
            personality: "caring".into(),
            scenario: "a forest glade".into(),
            example_dialogue: "<START>".into(),
            ..Default::default()
        }),
        persona: Some(PersonaV1 {
            name: "Traveler".into(),
            description: "A weary traveler".into(),
            ..Default::default()
        }),
        world_books: vec![WorldBookV1 {
            name: "Eldoria".into(),
            entries: vec![
                entry("eldoria", "Eldoria is the forest.", &["eldoria"]),
                entry("unused", "Never mentioned.", &["dragon"]),
                WorldBookEntry {
                    constant: true,
                    ..entry("always", "The air smells of moss.", &[])
                },
            ],
            ..Default::default()
        }],
        preset: OpenAiChatPresetV1 {
            prompts: vec![
                prompt("main", "You are {{char}}, talking to {{user}}.", None),
                prompt("wiBefore", "", Some(PromptMarker::WorldInfoBefore)),
                prompt("charDesc", "", Some(PromptMarker::CharacterDescription)),
                prompt("history", "", Some(PromptMarker::ChatHistory)),
                prompt("disabled", "must not appear", None),
                PromptItem {
                    injection: PromptInjection::AtDepth {
                        depth: 1,
                        order: 100,
                        triggers: Vec::new(),
                    },
                    ..prompt("jailbreak", "Stay in character.", None)
                },
            ],
            prompt_orders: vec![PromptOrderProfile {
                name: "default".into(),
                entries: ["main", "wiBefore", "charDesc", "history", "jailbreak"]
                    .iter()
                    .map(|identifier| PromptOrderEntry {
                        identifier: (*identifier).to_owned(),
                        enabled: true,
                    })
                    .chain(std::iter::once(PromptOrderEntry {
                        identifier: "disabled".to_owned(),
                        enabled: false,
                    }))
                    .collect(),
            }],
            context: ContextAssembly::default(),
            ..Default::default()
        },
        history: vec![
            HistoryTurn {
                role: TurnRole::User,
                text: "Tell me about Eldoria".into(),
            },
            HistoryTurn {
                role: TurnRole::Assistant,
                text: "It is my home.".into(),
            },
        ],
        user_message: Some("And you, {{char}}?".into()),
        model: ModelId("gpt-4.1-mini".into()),
        ..Default::default()
    }
}

#[test]
fn builds_a_sillytavern_shaped_context() {
    let snapshot = snapshot();
    let draft = build_context(&snapshot);

    // 系统提示词进 instructions,宏已展开。
    let system: Vec<String> = draft
        .instructions
        .iter()
        .map(|instruction| match &instruction.content[0] {
            guanfu_core::llm::ir::generation::InputContent::Text { text } => text.clone(),
            _ => unreachable!("fixture only builds text"),
        })
        .collect();
    assert!(system[0].contains("You are Seraphina, talking to Traveler."));
    // 关键词命中与常驻条目激活,未命中的不进上下文。
    assert!(system[1].contains("Eldoria is the forest."));
    assert!(system[1].contains("The air smells of moss."));
    assert!(!system.concat().contains("Never mentioned."));
    assert!(system[2].contains("Guardian of Traveler's glade"));
    assert!(!system.concat().contains("must not appear"));

    let texts: Vec<&str> = draft
        .messages
        .iter()
        .map(|message| message.text.as_str())
        .collect();
    // 历史 → 深度注入(depth 1 = 末条之前) → 本轮用户输入。
    assert_eq!(
        texts,
        [
            "Tell me about Eldoria",
            "Stay in character.",
            "It is my home.",
            "And you, Seraphina?",
        ]
    );

    // 纯函数:同一快照给出同一结果。
    assert_eq!(draft, build_context(&snapshot));
}
