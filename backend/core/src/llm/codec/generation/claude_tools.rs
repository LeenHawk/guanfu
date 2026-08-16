//! IR 工具定义 / tool_choice → Claude Messages 原生工具(对齐 gproxy
//! openai_responses_to_claude_messages/tool_builders.rs 的版本选择)。

use gproxy_protocol::claude as wire;
use gproxy_protocol::OperationKey;

use super::claude::{incompatible, wire_build_error};
use crate::llm::ir::generation::*;
use crate::CoreError;

pub(super) fn encode_tools(
    tools: &[ToolDefinition],
    target: OperationKey,
) -> Result<(Vec<wire::Tool>, Vec<wire::McpServer>), CoreError> {
    let mut output = Vec::new();
    let mut mcp_servers = Vec::new();
    for tool in tools {
        match tool {
            ToolDefinition::Function(tool) => output.push(custom_tool(
                tool.name.clone(),
                tool.description.clone(),
                schema(&tool.parameters),
                Some(tool.strict),
            )?),
            // grammar 输入格式在 Claude 无落点,与 transform 一致静默退化为空 schema。
            ToolDefinition::Custom(tool) => output.push(custom_tool(
                tool.name.clone(),
                tool.description.clone(),
                empty_schema()?,
                None,
            )?),
            ToolDefinition::WebSearch(tool) => output.push(web_search_tool(tool)?),
            ToolDefinition::WebFetch(tool) => output.push(web_fetch_tool(tool)?),
            ToolDefinition::TextEditor(tool) => output.push(wire::Tool::TextEditor(
                wire::TextEditorTool::TextEditor20250728(
                    wire::TextEditorTool20250728::builder()
                        .name(wire::StrReplaceBasedEditToolName::StrReplaceBasedEditTool)
                        .type_(wire::TextEditorTool20250728Type::TextEditor20250728)
                        .max_characters(tool.max_characters)
                        .common(Default::default())
                        .build()
                        .map_err(wire_build_error)?,
                ),
            )),
            ToolDefinition::Memory => {
                output.push(wire::Tool::Command(wire::CommandTool::Memory20250818(
                    wire::MemoryTool20250818::builder()
                        .name(wire::MemoryToolName::Memory)
                        .type_(wire::MemoryTool20250818Type::Memory20250818)
                        .common(Default::default())
                        .build()
                        .map_err(wire_build_error)?,
                )))
            }
            // container 选择由 Claude 服务端管理,静默丢弃。
            ToolDefinition::CodeExecution(_) => output.push(wire::Tool::Command(
                wire::CommandTool::CodeExecution20260120(
                    wire::CodeExecutionTool20260120::builder()
                        .name(wire::CodeExecutionToolName::CodeExecution)
                        .type_(wire::CodeExecutionTool20260120Type::CodeExecution20260120)
                        .common(Default::default())
                        .build()
                        .map_err(wire_build_error)?,
                ),
            )),
            ToolDefinition::Shell(_) => {
                output.push(wire::Tool::Command(wire::CommandTool::Bash20250124(
                    wire::BashTool20250124::builder()
                        .name(wire::BashToolName::Bash)
                        .type_(wire::BashTool20250124Type::Bash20250124)
                        .common(Default::default())
                        .build()
                        .map_err(wire_build_error)?,
                )))
            }
            ToolDefinition::ComputerUse(tool) => {
                output.push(wire::Tool::Computer(wire::ComputerTool::Computer20250124(
                    wire::ComputerTool20250124::builder()
                        .display_height_px(u64::from(tool.display_height))
                        .display_width_px(u64::from(tool.display_width))
                        .name(wire::ComputerToolName::Computer)
                        .type_(wire::ComputerTool20250124Type::Computer20250124)
                        .common(Default::default())
                        .build()
                        .map_err(wire_build_error)?,
                )))
            }
            // deferred_tools 列表无对应落点,静默丢弃(与 transform 一致)。
            ToolDefinition::ToolSearch(_) => {
                output.push(wire::Tool::Command(wire::CommandTool::ToolSearchBm25(
                    wire::ToolSearchBm25Tool::builder()
                        .name(wire::ToolSearchBm25ToolName::ToolSearchBm25)
                        .type_(wire::ToolSearchBm25ToolType::ToolSearchBm25)
                        .common(Default::default())
                        .build()
                        .map_err(wire_build_error)?,
                )))
            }
            ToolDefinition::Mcp(tool) => mcp_servers.push(mcp_server(tool)?),
            // OpenAI 托管语义在 Claude 无法表达,显式 route incompatible 参与回退。
            ToolDefinition::FileSearch(_) => {
                return Err(incompatible(target, "tools[].file_search"))
            }
            ToolDefinition::ImageGeneration(_) => {
                return Err(incompatible(target, "tools[].image_generation"))
            }
        }
    }
    Ok((output, mcp_servers))
}

fn custom_tool(
    name: String,
    description: Option<String>,
    input_schema: wire::JsonSchema,
    strict: Option<bool>,
) -> Result<wire::Tool, CoreError> {
    let mut common = wire::ToolCommon::default();
    common.strict = strict;
    let mut builder = wire::CustomTool::builder()
        .input_schema(input_schema)
        .name(name)
        .type_(Some(wire::CustomToolType::Custom))
        .common(common);
    if description.is_some() {
        builder = builder.description(description);
    }
    Ok(wire::Tool::Custom(
        builder.build().map_err(wire_build_error)?,
    ))
}

fn schema(schema: &crate::llm::ir::JsonSchema) -> wire::JsonSchema {
    serde_json::from_value(schema.0.clone())
        .unwrap_or_else(|_| empty_schema().expect("empty schema builds"))
}

fn empty_schema() -> Result<wire::JsonSchema, CoreError> {
    wire::JsonSchema::builder()
        .type_(wire::JsonSchemaObjectType::Known(
            wire::JsonSchemaObjectTypeKnown::Object,
        ))
        .build()
        .map_err(wire_build_error)
}

/// search_context_size 与经纬度在 Claude 无落点,静默丢弃。
fn web_search_tool(tool: &WebSearchTool) -> Result<wire::Tool, CoreError> {
    let params = wire::WebSearchToolParams::builder()
        .allowed_domains(non_empty(&tool.allowed_domains))
        .blocked_domains(non_empty(&tool.blocked_domains))
        .max_uses(tool.max_uses.map(u64::from))
        .user_location(tool.user_location.as_ref().map(user_location).transpose()?)
        .build()
        .map_err(wire_build_error)?;
    Ok(wire::Tool::WebSearch(
        wire::WebSearchTool::WebSearch20260209(
            wire::WebSearchTool20260209::builder()
                .name(wire::WebSearchToolName::WebSearch)
                .type_(wire::WebSearchTool20260209Type::WebSearch20260209)
                .params(params)
                .common(Default::default())
                .build()
                .map_err(wire_build_error)?,
        ),
    ))
}

fn web_fetch_tool(tool: &WebFetchTool) -> Result<wire::Tool, CoreError> {
    let params = wire::WebFetchToolParams::builder()
        .allowed_domains(non_empty(&tool.allowed_domains))
        .blocked_domains(non_empty(&tool.blocked_domains))
        .max_uses(tool.max_uses.map(u64::from))
        .build()
        .map_err(wire_build_error)?;
    Ok(wire::Tool::WebFetch(wire::WebFetchTool::WebFetch20250910(
        wire::WebFetchTool20250910::builder()
            .name(wire::WebFetchToolName::WebFetch)
            .type_(wire::WebFetchTool20250910Type::WebFetch20250910)
            .params(params)
            .common(Default::default())
            .build()
            .map_err(wire_build_error)?,
    )))
}

fn user_location(location: &UserLocation) -> Result<wire::UserLocation, CoreError> {
    let mut builder = wire::UserLocation::builder().type_(wire::UserLocationType::Approximate);
    builder = builder.city(location.city.clone());
    builder = builder.country(location.country.clone());
    builder = builder.region(location.region.clone());
    builder = builder.timezone(location.timezone.clone());
    builder.build().map_err(wire_build_error)
}

/// MCP 审批策略在 Claude 无落点,静默丢弃(与 transform 一致)。
fn mcp_server(tool: &McpTool) -> Result<wire::McpServer, CoreError> {
    let configuration = (!tool.allowed_tools.is_empty()).then(|| {
        wire::McpToolConfiguration::builder()
            .allowed_tools(Some(tool.allowed_tools.clone()))
            .build()
            .expect("complete MCP tool configuration")
    });
    let mut builder = wire::McpServer::builder()
        .name(tool.server_label.clone())
        .type_(wire::McpServerType::Known(wire::McpServerTypeKnown::Url))
        .url(tool.server_url.clone());
    if configuration.is_some() {
        builder = builder.tool_configuration(configuration);
    }
    builder.build().map_err(wire_build_error)
}

pub(super) fn encode_tool_choice(choice: &ToolChoice) -> Result<wire::ToolChoice, CoreError> {
    Ok(match choice {
        ToolChoice::Auto => wire::ToolChoice::Auto(
            wire::ToolChoiceAuto::builder()
                .type_(wire::ToolChoiceAutoType::Auto)
                .build()
                .map_err(wire_build_error)?,
        ),
        ToolChoice::None => wire::ToolChoice::None(
            wire::ToolChoiceNone::builder()
                .type_(wire::ToolChoiceNoneType::None)
                .build()
                .map_err(wire_build_error)?,
        ),
        ToolChoice::Required => any_choice()?,
        ToolChoice::Tool { name } => named_choice(name.clone())?,
        // 与 transform 一致:单个白名单工具收敛为指名,多个退化为 any。
        ToolChoice::Allowed { names, .. } => match names.as_slice() {
            [name] => named_choice(name.clone())?,
            _ => any_choice()?,
        },
    })
}

fn any_choice() -> Result<wire::ToolChoice, CoreError> {
    Ok(wire::ToolChoice::Any(
        wire::ToolChoiceAny::builder()
            .type_(wire::ToolChoiceAnyType::Any)
            .build()
            .map_err(wire_build_error)?,
    ))
}

fn named_choice(name: String) -> Result<wire::ToolChoice, CoreError> {
    Ok(wire::ToolChoice::Tool(
        wire::ToolChoiceTool::builder()
            .name(name)
            .type_(wire::ToolChoiceToolType::Tool)
            .build()
            .map_err(wire_build_error)?,
    ))
}

fn non_empty(values: &[String]) -> Option<Vec<String>> {
    (!values.is_empty()).then(|| values.to_vec())
}
