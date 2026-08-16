//! 宏替换:ST 宏的必要子集。
//!
//! 只做名称与角色文本类替换;时间、随机、脚本类宏后置(计划 §10)。

use super::ContextSnapshot;

pub struct MacroContext {
    char_name: String,
    user_name: String,
    description: String,
    personality: String,
    scenario: String,
    persona: String,
}

impl MacroContext {
    pub fn from_snapshot(snapshot: &ContextSnapshot) -> Self {
        let character = snapshot.character.as_ref();
        let persona = snapshot.persona.as_ref();
        Self {
            char_name: character.map(|c| c.name.clone()).unwrap_or_default(),
            user_name: persona.map(|p| p.name.clone()).unwrap_or_default(),
            description: character.map(|c| c.description.clone()).unwrap_or_default(),
            personality: character.map(|c| c.personality.clone()).unwrap_or_default(),
            scenario: character.map(|c| c.scenario.clone()).unwrap_or_default(),
            persona: persona.map(|p| p.description.clone()).unwrap_or_default(),
        }
    }

    pub fn expand(&self, text: &str) -> String {
        if !text.contains("{{") {
            return text.to_owned();
        }
        let mut output = text.to_owned();
        for (name, value) in [
            ("char", &self.char_name),
            ("user", &self.user_name),
            ("description", &self.description),
            ("personality", &self.personality),
            ("scenario", &self.scenario),
            ("persona", &self.persona),
        ] {
            if !output.contains("{{") {
                break;
            }
            // ST 的宏名大小写不敏感。
            output = replace_macro(&output, name, value);
        }
        output
    }
}

fn replace_macro(text: &str, name: &str, value: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let Some(end) = rest[start..].find("}}") else {
            break;
        };
        let end = start + end;
        let token = rest[start + 2..end].trim();
        output.push_str(&rest[..start]);
        if token.eq_ignore_ascii_case(name) {
            output.push_str(value);
        } else {
            output.push_str(&rest[start..end + 2]);
        }
        rest = &rest[end + 2..];
    }
    output.push_str(rest);
    output
}
