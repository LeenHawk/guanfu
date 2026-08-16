//! IR 工具定义 / tool_choice → Gemini 原生工具(对照 gproxy
//! openai_responses_to_gemini_generate_content/tools.rs 及其反向映射)。

use gproxy_protocol::gemini as wire;
use gproxy_protocol::OperationKey;
use serde_json::Value;

use super::claude::{incompatible, wire_build_error};
use crate::llm::ir::generation::*;
use crate::CoreError;

pub(super) fn encode_tools(
    tools: &[ToolDefinition],
    target: OperationKey,
) -> Result<Vec<wire::Tool>, CoreError> {
    let mut declarations = Vec::new();
    let mut output = Vec::new();
    for tool in tools {
        match tool {
            // strict 无落点,静默丢弃(与 gproxy 一致)。
            ToolDefinition::Function(tool) => declarations.push(declaration(
                tool.name.clone(),
                tool.description.clone(),
                Some(tool.parameters.0.clone()),
            )?),
            // grammar 输入格式无落点,退化为无参函数(与 gproxy 一致)。
            ToolDefinition::Custom(tool) => declarations.push(declaration(
                tool.name.clone(),
                tool.description.clone(),
                None,
            )?),
            // 域名过滤/max_uses/user_location/context size 无落点,静默丢弃。
            ToolDefinition::WebSearch(_) => output.push(
                wire::Tool::builder()
                    .google_search(Some(wire::GoogleSearch::default()))
                    .url_context(Some(wire::UrlContext::default()))
                    .build()
                    .map_err(wire_build_error)?,
            ),
            // container 由服务端管理,静默丢弃。
            ToolDefinition::CodeExecution(_) => output.push(
                wire::Tool::builder()
                    .code_execution(Some(wire::CodeExecution::default()))
                    .build()
                    .map_err(wire_build_error)?,
            ),
            ToolDefinition::ComputerUse(tool) => output.push(computer_use(tool, target)?),
            // allowed_tools 与审批策略无落点,静默丢弃(与 gproxy 一致)。
            ToolDefinition::Mcp(tool) => output.push(mcp_tool(tool)?),
            // 其余托管语义在 Gemini 无法表达,显式 route incompatible 参与回退。
            ToolDefinition::WebFetch(_) => return Err(incompatible(target, "tools[].web_fetch")),
            ToolDefinition::FileSearch(_) => {
                return Err(incompatible(target, "tools[].file_search"))
            }
            ToolDefinition::Shell(_) => return Err(incompatible(target, "tools[].shell")),
            ToolDefinition::TextEditor(_) => {
                return Err(incompatible(target, "tools[].text_editor"))
            }
            ToolDefinition::ImageGeneration(_) => {
                return Err(incompatible(target, "tools[].image_generation"))
            }
            ToolDefinition::Memory => return Err(incompatible(target, "tools[].memory")),
            ToolDefinition::ToolSearch(_) => {
                return Err(incompatible(target, "tools[].tool_search"))
            }
        }
    }
    if !declarations.is_empty() {
        output.insert(
            0,
            wire::Tool::builder()
                .function_declarations(declarations)
                .build()
                .map_err(wire_build_error)?,
        );
    }
    Ok(output)
}

fn declaration(
    name: String,
    description: Option<String>,
    schema: Option<Value>,
) -> Result<wire::FunctionDeclaration, CoreError> {
    wire::FunctionDeclaration::builder()
        .name(name)
        .description(description.unwrap_or_default())
        .parameters_json_schema(schema)
        .build()
        .map_err(wire_build_error)
}

/// display 尺寸无落点,静默丢弃;Gemini 只支持 browser 环境,其余显式回退。
fn computer_use(tool: &ComputerUseTool, target: OperationKey) -> Result<wire::Tool, CoreError> {
    if tool.environment != ComputerEnvironment::Browser {
        return Err(incompatible(target, "tools[].computer_use.environment"));
    }
    wire::Tool::builder()
        .computer_use(Some(
            wire::ComputerUse::builder()
                .environment(Some(wire::ComputerUseEnvironment::Known(
                    wire::ComputerUseEnvironmentKnown::EnvironmentBrowser,
                )))
                .build()
                .map_err(wire_build_error)?,
        ))
        .build()
        .map_err(wire_build_error)
}

fn mcp_tool(tool: &McpTool) -> Result<wire::Tool, CoreError> {
    let transport = wire::StreamableHttpTransport::builder()
        .url(Some(tool.server_url.clone()))
        .build()
        .map_err(wire_build_error)?;
    let server = wire::McpServer::builder()
        .name(Some(tool.server_label.clone()))
        .streamable_http_transport(Some(transport))
        .build()
        .map_err(wire_build_error)?;
    wire::Tool::builder()
        .mcp_servers(vec![server])
        .build()
        .map_err(wire_build_error)
}

/// tool_choice → toolConfig.functionCallingConfig(与 gproxy 一致:
/// Required/指名 → ANY,Allowed 白名单全量保留)。
pub(super) fn encode_tool_choice(choice: &ToolChoice) -> Result<wire::ToolConfig, CoreError> {
    let (mode, names) = match choice {
        ToolChoice::Auto => (wire::FunctionCallingModeKnown::Auto, Vec::new()),
        ToolChoice::None => (wire::FunctionCallingModeKnown::None, Vec::new()),
        ToolChoice::Required => (wire::FunctionCallingModeKnown::Any, Vec::new()),
        ToolChoice::Tool { name } => (wire::FunctionCallingModeKnown::Any, vec![name.clone()]),
        ToolChoice::Allowed { names, mode } => (
            match mode {
                AllowedToolMode::Auto => wire::FunctionCallingModeKnown::Auto,
                AllowedToolMode::Required => wire::FunctionCallingModeKnown::Any,
            },
            names.clone(),
        ),
    };
    let config = wire::FunctionCallingConfig::builder()
        .mode(Some(wire::FunctionCallingMode::Known(mode)))
        .allowed_function_names(names)
        .build()
        .map_err(wire_build_error)?;
    wire::ToolConfig::builder()
        .function_calling_config(Some(config))
        .build()
        .map_err(wire_build_error)
}
