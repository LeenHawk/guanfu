//! IR 工具定义 / tool_choice → Chat Completions 原生工具(对照 gproxy
//! openai_responses_to_openai_chat/tools.rs 现行为)。

use std::collections::BTreeMap;

use gproxy_protocol::openai as wire;
use gproxy_protocol::OperationKey;
use serde_json::json;

use super::claude::{incompatible, wire_build_error};
use crate::llm::ir::generation::*;
use crate::CoreError;

#[derive(Default)]
pub(super) struct ChatTools {
    pub(super) tools: Vec<wire::ChatTool>,
    pub(super) web_search_options: Option<wire::ChatWebSearchOptions>,
}

/// function/custom 直写;web_search 折到 web_search_options;其余托管语义
/// 在 Chat 无落点,显式 route incompatible 参与回退(旧两跳为静默丢弃)。
/// web_fetch / memory / web_search 限额已由 validate_target_tools 先行拦截。
pub(super) fn encode_tools(
    tools: &[ToolDefinition],
    target: OperationKey,
) -> Result<ChatTools, CoreError> {
    let mut output = ChatTools::default();
    for tool in tools {
        match tool {
            ToolDefinition::Function(tool) => output.tools.push(wire::ChatTool::Function {
                function: wire::FunctionDefinition::builder()
                    .name(tool.name.clone())
                    .description(tool.description.clone())
                    .parameters(Some(
                        serde_json::from_value(tool.parameters.0.clone()).unwrap_or_default(),
                    ))
                    .strict(Some(tool.strict))
                    .build()
                    .map_err(wire_build_error)?,
                extra: Default::default(),
            }),
            ToolDefinition::Custom(tool) => output.tools.push(wire::ChatTool::Custom {
                custom: wire::CustomToolDefinition::builder()
                    .name(tool.name.clone())
                    .description(tool.description.clone())
                    .format(custom_format(&tool.input_format)?)
                    .build()
                    .map_err(wire_build_error)?,
                extra: Default::default(),
            }),
            ToolDefinition::WebSearch(tool) => {
                output.web_search_options = Some(web_search_options(tool, target)?)
            }
            ToolDefinition::WebFetch(_) => return Err(incompatible(target, "tools[].web_fetch")),
            ToolDefinition::FileSearch(_) => {
                return Err(incompatible(target, "tools[].file_search"))
            }
            ToolDefinition::ComputerUse(_) => {
                return Err(incompatible(target, "tools[].computer_use"))
            }
            ToolDefinition::CodeExecution(_) => {
                return Err(incompatible(target, "tools[].code_execution"))
            }
            ToolDefinition::Shell(_) => return Err(incompatible(target, "tools[].shell")),
            ToolDefinition::TextEditor(_) => {
                return Err(incompatible(target, "tools[].text_editor"))
            }
            ToolDefinition::ImageGeneration(_) => {
                return Err(incompatible(target, "tools[].image_generation"))
            }
            ToolDefinition::Mcp(_) => return Err(incompatible(target, "tools[].mcp")),
            ToolDefinition::Memory => return Err(incompatible(target, "tools[].memory")),
            ToolDefinition::ToolSearch(_) => {
                return Err(incompatible(target, "tools[].tool_search"))
            }
        }
    }
    Ok(output)
}

/// Text 输入格式缺省不发(与 gproxy 现行为一致),grammar 直写扁平 wire 形。
fn custom_format(
    format: &CustomToolInputFormat,
) -> Result<Option<wire::CustomToolInputFormat>, CoreError> {
    Ok(match format {
        CustomToolInputFormat::Text => None,
        CustomToolInputFormat::Grammar { syntax, definition } => {
            Some(wire::CustomToolInputFormat::Grammar(
                wire::CustomToolGrammarFormat::builder()
                    .type_(wire::CustomToolGrammarFormatType::Grammar)
                    .grammar(
                        wire::CustomToolGrammar::builder()
                            .definition(definition.clone())
                            .syntax(match syntax {
                                GrammarSyntax::Lark => wire::CustomToolGrammarSyntax::Lark,
                                GrammarSyntax::Regex => wire::CustomToolGrammarSyntax::Regex,
                            })
                            .build()
                            .map_err(wire_build_error)?,
                    )
                    .build()
                    .map_err(wire_build_error)?,
            ))
        }
    })
}

/// allowed_domains 在 Chat 无落点显式回退;经纬度无落点静默丢弃(同 Claude codec)。
fn web_search_options(
    tool: &WebSearchTool,
    target: OperationKey,
) -> Result<wire::ChatWebSearchOptions, CoreError> {
    if !tool.allowed_domains.is_empty() {
        return Err(incompatible(target, "tools[].web_search.allowed_domains"));
    }
    wire::ChatWebSearchOptions::builder()
        .search_context_size(tool.search_context_size.map(|size| match size {
            SearchContextSize::Low => wire::SearchContextSize::Low,
            SearchContextSize::Medium => wire::SearchContextSize::Medium,
            SearchContextSize::High => wire::SearchContextSize::High,
        }))
        .user_location(tool.user_location.as_ref().map(user_location).transpose()?)
        .build()
        .map_err(wire_build_error)
}

fn user_location(location: &UserLocation) -> Result<wire::ChatWebSearchUserLocation, CoreError> {
    wire::ChatWebSearchUserLocation::builder()
        .approximate(
            wire::ApproximateLocation::builder()
                .city(location.city.clone())
                .country(location.country.clone())
                .region(location.region.clone())
                .timezone(location.timezone.clone())
                .build()
                .map_err(wire_build_error)?,
        )
        .type_(wire::ApproximateLocationType::Approximate)
        .build()
        .map_err(wire_build_error)
}

pub(super) fn encode_tool_choice(
    choice: &ToolChoice,
    tools: &[ToolDefinition],
) -> Result<wire::ChatToolChoice, CoreError> {
    Ok(match choice {
        ToolChoice::Auto => wire::ChatToolChoice::Mode(wire::ToolChoiceMode::Auto),
        ToolChoice::None => wire::ChatToolChoice::Mode(wire::ToolChoiceMode::None),
        ToolChoice::Required => wire::ChatToolChoice::Mode(wire::ToolChoiceMode::Required),
        ToolChoice::Tool { name } => wire::ChatToolChoice::Named(named_choice(name, tools)?),
        // 与 gproxy 不同:Chat wire 支持 allowed_tools,直写而非静默丢弃。
        ToolChoice::Allowed { names, mode } => wire::ChatToolChoice::Allowed(
            wire::ChatAllowedToolChoice::builder()
                .allowed_tools(
                    wire::ChatAllowedTools::builder()
                        .mode(match mode {
                            AllowedToolMode::Auto => wire::AllowedToolsMode::Auto,
                            AllowedToolMode::Required => wire::AllowedToolsMode::Required,
                        })
                        .tools(
                            names
                                .iter()
                                .map(|name| allowed_entry(name, tools))
                                .collect(),
                        )
                        .build()
                        .map_err(wire_build_error)?,
                )
                .type_(wire::AllowedToolsType::AllowedTools)
                .build()
                .map_err(wire_build_error)?,
        ),
    })
}

fn named_choice(
    name: &str,
    tools: &[ToolDefinition],
) -> Result<wire::ChatNamedToolChoice, CoreError> {
    let named = wire::NamedTool::builder()
        .name(name.to_owned())
        .build()
        .map_err(wire_build_error)?;
    Ok(if is_custom_tool(name, tools) {
        wire::ChatNamedToolChoice::Custom {
            type_: wire::CustomToolChoiceType::Custom,
            custom: named,
            extra: Default::default(),
        }
    } else {
        wire::ChatNamedToolChoice::Function {
            type_: wire::FunctionToolChoiceType::Function,
            function: named,
            extra: Default::default(),
        }
    })
}

/// 白名单条目按定义列表还原工具类型;未知名字按 function 兜底(同 canonical)。
fn allowed_entry(name: &str, tools: &[ToolDefinition]) -> BTreeMap<String, serde_json::Value> {
    let (kind, key) = if is_custom_tool(name, tools) {
        ("custom", "custom")
    } else {
        ("function", "function")
    };
    BTreeMap::from([
        ("type".to_owned(), json!(kind)),
        (key.to_owned(), json!({ "name": name })),
    ])
}

fn is_custom_tool(name: &str, tools: &[ToolDefinition]) -> bool {
    tools
        .iter()
        .any(|tool| matches!(tool, ToolDefinition::Custom(tool) if tool.name == name))
}
