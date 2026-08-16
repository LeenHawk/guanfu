//! ContextBuild:从 run 的 Asset 快照到一次生成草稿的纯函数。
//!
//! 数据库查询与 Asset 解码在 snapshot loader 完成;这里不做 I/O、不调模型
//! (需要模型调用的 compact 不能藏进来),因此同一份快照永远得到同一份
//! 草稿——run 的可复现性依赖这一点(计划 §7)。

mod macros;
mod world_book;

use crate::assets::character::CharacterV1;
use crate::assets::persona::{PersonaPosition, PersonaV1};
use crate::assets::preset::{
    GenerationTrigger, OpenAiChatPresetV1, PromptInjection, PromptItem, PromptMarker,
};
use crate::assets::world_book::WorldBookV1;
use crate::llm::ir::generation::{Instruction, InstructionRole, ReasoningOptions, SamplingOptions};
use crate::llm::ir::ModelId;

pub use macros::MacroContext;
pub use world_book::{activate, ActivatedEntry};

/// runner 装载后的只读快照。
#[derive(Clone, Debug)]
pub struct ContextSnapshot {
    pub character: Option<CharacterV1>,
    pub persona: Option<PersonaV1>,
    pub world_books: Vec<WorldBookV1>,
    pub preset: OpenAiChatPresetV1,
    /// 已解码的历史消息(纯文本视图,足够本阶段编排)。
    pub history: Vec<HistoryTurn>,
    /// 本轮用户输入;continue / regenerate 等触发可以没有。
    pub user_message: Option<String>,
    pub trigger: GenerationTrigger,
    pub model: ModelId,
    /// 概率激活的确定性掷点:同一 run 重放得到同一结果。
    pub activation_seed: u64,
}

impl Default for ContextSnapshot {
    fn default() -> Self {
        Self {
            character: None,
            persona: None,
            world_books: Vec::new(),
            preset: OpenAiChatPresetV1::default(),
            history: Vec::new(),
            user_message: None,
            trigger: GenerationTrigger::Normal,
            model: ModelId(String::new()),
            activation_seed: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryTurn {
    pub role: TurnRole,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnRole {
    User,
    Assistant,
    System,
}

/// 编排结果:直接喂给 Generate 节点。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GenerationDraft {
    pub instructions: Vec<Instruction>,
    pub messages: Vec<DraftMessage>,
    pub sampling: SamplingOptions,
    pub reasoning: Option<ReasoningOptions>,
    pub max_output_tokens: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftMessage {
    pub role: TurnRole,
    pub text: String,
}

pub fn build_context(snapshot: &ContextSnapshot) -> GenerationDraft {
    let macros = MacroContext::from_snapshot(snapshot);
    let activated = world_book::activate(snapshot, &macros);

    let order = snapshot
        .preset
        .prompt_orders
        .first()
        .map(|profile| profile.entries.clone())
        .unwrap_or_default();
    let enabled: Vec<&PromptItem> = if order.is_empty() {
        // 没有 order profile 时按声明顺序全用。
        snapshot.preset.prompts.iter().collect()
    } else {
        order
            .iter()
            .filter(|entry| entry.enabled)
            .filter_map(|entry| {
                snapshot
                    .preset
                    .prompts
                    .iter()
                    .find(|prompt| prompt.identifier == entry.identifier)
            })
            .collect()
    };

    let mut instructions = Vec::new();
    let mut messages = Vec::new();
    let mut depth_injections: Vec<(u32, i32, DraftMessage)> = Vec::new();

    for prompt in enabled {
        if let PromptInjection::AtDepth {
            depth,
            order,
            triggers,
        } = &prompt.injection
        {
            if !triggers.is_empty() && !triggers.contains(&snapshot.trigger) {
                continue;
            }
            let text = macros.expand(&prompt.content);
            if !text.is_empty() || snapshot.preset.context.send_if_empty {
                depth_injections.push((
                    *depth,
                    *order,
                    DraftMessage {
                        role: role_of(prompt),
                        text,
                    },
                ));
            }
            continue;
        }
        emit_prompt(
            snapshot,
            &macros,
            &activated,
            prompt,
            &mut instructions,
            &mut messages,
        );
    }

    // 深度注入:depth 从历史末尾往前数,同深度按 order 稳定。
    depth_injections.sort_by_key(|(depth, order, _)| (std::cmp::Reverse(*depth), *order));
    for (depth, _, message) in depth_injections {
        let index = messages.len().saturating_sub(depth as usize);
        messages.insert(index, message);
    }

    if let Some(user_message) = &snapshot.user_message {
        messages.push(DraftMessage {
            role: TurnRole::User,
            text: macros.expand(user_message),
        });
    }

    GenerationDraft {
        instructions,
        messages,
        sampling: snapshot.preset.sampling.clone(),
        reasoning: snapshot.preset.reasoning.clone(),
        max_output_tokens: snapshot.preset.context.max_output_tokens.map(u64::from),
    }
}

fn emit_prompt(
    snapshot: &ContextSnapshot,
    macros: &MacroContext,
    activated: &[ActivatedEntry],
    prompt: &PromptItem,
    instructions: &mut Vec<Instruction>,
    messages: &mut Vec<DraftMessage>,
) {
    let text = match prompt.marker {
        None => macros.expand(&prompt.content),
        Some(PromptMarker::CharacterDescription) => macros.expand(
            &snapshot
                .character
                .as_ref()
                .map(|character| character.description.clone())
                .unwrap_or_default(),
        ),
        Some(PromptMarker::CharacterPersonality) => macros.expand(
            &snapshot
                .character
                .as_ref()
                .map(|character| character.personality.clone())
                .unwrap_or_default(),
        ),
        Some(PromptMarker::Scenario) => macros.expand(
            &snapshot
                .character
                .as_ref()
                .map(|character| character.scenario.clone())
                .unwrap_or_default(),
        ),
        Some(PromptMarker::PersonaDescription) => {
            match snapshot.persona.as_ref() {
                // NONE 位置的 persona 描述不进提示词。
                Some(persona) if persona.position != PersonaPosition::None => {
                    macros.expand(&persona.description)
                }
                _ => String::new(),
            }
        }
        Some(PromptMarker::DialogueExamples) => macros.expand(
            &snapshot
                .character
                .as_ref()
                .map(|character| character.example_dialogue.clone())
                .unwrap_or_default(),
        ),
        Some(PromptMarker::WorldInfoBefore) => {
            world_book::render(activated, world_book::Slot::Before)
        }
        Some(PromptMarker::WorldInfoAfter) => {
            world_book::render(activated, world_book::Slot::After)
        }
        Some(PromptMarker::ChatHistory) => {
            messages.extend(snapshot.history.iter().map(|turn| DraftMessage {
                role: turn.role,
                text: turn.text.clone(),
            }));
            return;
        }
    };
    if text.is_empty() && !snapshot.preset.context.send_if_empty {
        return;
    }
    // 历史之前的 system 提示词进 instructions,其余按消息追加。
    if role_of(prompt) == TurnRole::System && messages.is_empty() {
        instructions.push(Instruction {
            role: InstructionRole::System,
            content: vec![crate::llm::ir::generation::InputContent::Text { text }],
        });
    } else {
        messages.push(DraftMessage {
            role: role_of(prompt),
            text,
        });
    }
}

fn role_of(prompt: &PromptItem) -> TurnRole {
    use crate::assets::character::InjectionRole;
    match prompt.role {
        InjectionRole::System => TurnRole::System,
        InjectionRole::User => TurnRole::User,
        InjectionRole::Assistant => TurnRole::Assistant,
    }
}
