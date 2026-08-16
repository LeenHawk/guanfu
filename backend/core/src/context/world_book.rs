//! 世界书激活:关键词扫描、选择逻辑、概率与预算。
//!
//! 概率用确定性掷点(种子来自 run),因此 build_context 仍是纯函数,
//! 同一个 run 重放得到同一批条目。

use super::{ContextSnapshot, MacroContext};
use crate::assets::world_book::{EntryPosition, SelectiveLogic, WorldBookEntry};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Before,
    After,
}

#[derive(Clone, Debug)]
pub struct ActivatedEntry {
    pub content: String,
    pub position: EntryPosition,
    pub order: i32,
}

/// 扫描历史尾部,返回按 order 排序、已过预算的条目。
pub fn activate(snapshot: &ContextSnapshot, macros: &MacroContext) -> Vec<ActivatedEntry> {
    let mut candidates: Vec<(usize, &WorldBookEntry, Option<u32>, Option<u32>)> = Vec::new();
    for book in &snapshot.world_books {
        for entry in &book.entries {
            candidates.push((candidates.len(), entry, book.scan_depth, book.token_budget));
        }
    }

    let budget = snapshot
        .world_books
        .iter()
        .filter_map(|book| book.token_budget)
        .min();

    let mut activated: Vec<ActivatedEntry> = Vec::new();
    for (index, entry, scan_depth, _) in &candidates {
        if !entry.enabled {
            continue;
        }
        if !entry.constant && !matches(entry, snapshot, *scan_depth) {
            continue;
        }
        if let Some(probability) = entry.probability {
            if roll(snapshot.activation_seed, *index) >= u32::from(probability) {
                continue;
            }
        }
        activated.push(ActivatedEntry {
            content: macros.expand(&entry.content),
            position: entry.position,
            order: entry.order,
        });
    }

    // order 小的先插入;同 order 保持声明顺序(sort_by_key 稳定)。
    activated.sort_by_key(|entry| entry.order);
    if let Some(budget) = budget {
        apply_budget(&mut activated, &snapshot.model.0, u64::from(budget));
    }
    activated
}

pub fn render(activated: &[ActivatedEntry], slot: Slot) -> String {
    activated
        .iter()
        .filter(|entry| match slot {
            Slot::Before => matches!(
                entry.position,
                EntryPosition::BeforeCharacter
                    | EntryPosition::TopAuthorNote
                    | EntryPosition::TopExampleMessages
            ),
            Slot::After => matches!(
                entry.position,
                EntryPosition::AfterCharacter
                    | EntryPosition::BottomAuthorNote
                    | EntryPosition::BottomExampleMessages
                    | EntryPosition::AtDepth
            ),
        })
        .map(|entry| entry.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn matches(entry: &WorldBookEntry, snapshot: &ContextSnapshot, scan_depth: Option<u32>) -> bool {
    let depth = scan_depth.unwrap_or(DEFAULT_SCAN_DEPTH) as usize;
    let mut haystack = snapshot
        .history
        .iter()
        .rev()
        .take(depth)
        .map(|turn| turn.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(user_message) = &snapshot.user_message {
        haystack.push('\n');
        haystack.push_str(user_message);
    }
    if !entry.case_sensitive {
        haystack = haystack.to_lowercase();
    }

    let primary = entry.keys.iter().any(|key| contains(&haystack, key, entry));
    if !primary {
        return false;
    }
    if entry.secondary_keys.is_empty() {
        return true;
    }
    let any_secondary = entry
        .secondary_keys
        .iter()
        .any(|key| contains(&haystack, key, entry));
    let all_secondary = entry
        .secondary_keys
        .iter()
        .all(|key| contains(&haystack, key, entry));
    match entry.selective_logic {
        SelectiveLogic::AndAny => any_secondary,
        SelectiveLogic::NotAll => !all_secondary,
        SelectiveLogic::NotAny => !any_secondary,
        SelectiveLogic::AndAll => all_secondary,
    }
}

fn contains(haystack: &str, key: &str, entry: &WorldBookEntry) -> bool {
    let needle = if entry.case_sensitive {
        key.to_owned()
    } else {
        key.to_lowercase()
    };
    if needle.is_empty() {
        return false;
    }
    if !entry.match_whole_words {
        return haystack.contains(&needle);
    }
    haystack
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word == needle)
}

/// 超出预算的条目从末尾丢弃(order 大的优先级低)。
fn apply_budget(activated: &mut Vec<ActivatedEntry>, model: &str, budget: u64) {
    let mut used = 0;
    let mut keep = 0;
    for entry in activated.iter() {
        let cost = crate::llm::count_tokens_local(model, entry.content.as_bytes());
        if used + cost > budget {
            break;
        }
        used += cost;
        keep += 1;
    }
    activated.truncate(keep);
}

const DEFAULT_SCAN_DEPTH: u32 = 4;

/// 与 run 绑定的确定性掷点(0-99)。
fn roll(seed: u64, index: usize) -> u32 {
    let mut value = seed ^ (index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    value ^= value >> 33;
    value = value.wrapping_mul(0xff51_afd7_ed55_8ccd);
    value ^= value >> 33;
    (value % 100) as u32
}
