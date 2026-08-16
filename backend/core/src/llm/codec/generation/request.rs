use base64::Engine;
use serde_json::{json, Value};

use super::*;
use crate::llm::ir::generation::*;
use crate::llm::ir::{FileId, MediaSource};

pub(super) fn encode_request(
    request: &GenerateRequest,
    target: OperationKey,
) -> Result<Value, CoreError> {
    let mut body = Map::new();
    body.insert("model".into(), json!(request.model.0));
    body.insert(
        "input".into(),
        encode_input(&request.instructions, &request.input)?,
    );
    if !request.tools.is_empty() {
        body.insert(
            "tools".into(),
            Value::Array(
                request
                    .tools
                    .iter()
                    .map(encode_tool)
                    .collect::<Result<_, _>>()?,
            ),
        );
    }
    body.insert(
        "tool_choice".into(),
        encode_tool_choice(&request.tool_choice, &request.tools),
    );
    body.insert("text".into(), encode_output_constraint(&request.output));
    if let Some(reasoning) = encode_reasoning(request.reasoning.as_ref()) {
        body.insert("reasoning".into(), reasoning);
    }
    insert_option(
        &mut body,
        "max_output_tokens",
        request.limits.max_output_tokens,
    );
    insert_option(&mut body, "max_tool_calls", request.limits.max_tool_calls);
    // Responses 线协议没有 modalities 参数:纯文本是缺省不发,
    // 非文本模态无落点,route incompatible 参与回退。
    let unsupported_modalities = request
        .modalities
        .iter()
        .filter(|modality| **modality != OutputModality::Text)
        .map(|modality| format!("modalities.{}", snake(modality)))
        .collect::<Vec<_>>();
    if !unsupported_modalities.is_empty() {
        return Err(CoreError::IncompatibleRoute {
            target,
            fields: unsupported_modalities,
        });
    }
    if request.mode == GenerateMode::Stream {
        body.insert("stream".into(), Value::Bool(true));
    }
    Ok(Value::Object(body))
}

pub(in crate::llm::codec) fn encode_input(
    instructions: &[Instruction],
    input: &[InputItem],
) -> Result<Value, CoreError> {
    let mut values = instructions
        .iter()
        .map(|instruction| {
            Ok(json!({
                "type": "message",
                "role": match instruction.role {
                    InstructionRole::System => "system",
                    InstructionRole::Developer => "developer",
                },
                "content": encode_content(&instruction.content)?,
            }))
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    values.extend(
        input
            .iter()
            .map(|item| match item {
                InputItem::Message { message } => Ok(json!({
                    "type": "message",
                    "role": match message.role {
                        MessageRole::System => "system",
                        MessageRole::User => "user",
                        MessageRole::Assistant => "assistant",
                    },
                    "content": encode_content(&message.content)?,
                })),
                InputItem::ToolResult { result } => encode_tool_result(result),
                InputItem::McpApproval { approval } => Ok(json!({
                    "type": "mcp_approval_response",
                    "approval_request_id": approval.approval_request_id,
                    "approve": approval.approve,
                })),
                InputItem::Reasoning { reasoning } => Ok(encode_reasoning_input(reasoning)),
            })
            .collect::<Result<Vec<_>, CoreError>>()?,
    );
    Ok(Value::Array(values))
}

fn encode_reasoning_input(reasoning: &ReasoningInput) -> Value {
    let mut item = Map::new();
    item.insert("type".into(), Value::String("reasoning".into()));
    item.insert("id".into(), json!(reasoning.previous.id.0));

    let mut summary = Vec::new();
    let mut content = Vec::new();
    let mut continuation = None;
    for part in &reasoning.previous.parts {
        match part {
            ReasoningPart::Summary { text } => {
                summary.push(json!({"type":"summary_text","text":text}));
            }
            ReasoningPart::Text {
                text,
                continuation: part_continuation,
            } => {
                content.push(json!({"type":"reasoning_text","text":text}));
                continuation = continuation.or(part_continuation.as_ref());
            }
            ReasoningPart::Opaque {
                continuation: part_continuation,
            } => {
                continuation = continuation.or(Some(part_continuation));
            }
        }
    }
    item.insert("summary".into(), Value::Array(summary));
    if !content.is_empty() {
        item.insert("content".into(), Value::Array(content));
    }
    if let Some(continuation) = continuation {
        item.insert(
            "encrypted_content".into(),
            Value::String(continuation.opaque_value().into()),
        );
    }
    Value::Object(item)
}

fn encode_content(content: &[InputContent]) -> Result<Value, CoreError> {
    Ok(Value::Array(
        content
            .iter()
            .map(|part| match part {
                InputContent::Text { text } => Ok(json!({"type":"input_text","text":text})),
                InputContent::Image { source, detail } => {
                    let (field, value) = encode_media_ref(source)?;
                    Ok(json!({"type":"input_image",field:value,"detail":snake(detail)}))
                }
                InputContent::Audio { source } => {
                    let MediaSource::Data { media_type, bytes } = source else {
                        return Err(CoreError::UnsupportedRouteImplementation { implementation: "Responses audio input requires inline data" });
                    };
                    let format = media_type.0.rsplit('/').next().unwrap_or("wav");
                    Ok(json!({"type":"input_audio","input_audio":{"data":base64::engine::general_purpose::STANDARD.encode(bytes),"format":format}}))
                }
                InputContent::File { source } => match source {
                    FileSource::Id { id } => Ok(json!({"type":"input_file","file_id":id.0})),
                    FileSource::Media { source } => {
                        let (field, value) = encode_file_ref(source)?;
                        Ok(json!({"type":"input_file",field:value}))
                    }
                    FileSource::Text { filename, text } => Ok(json!({"type":"input_file","filename":filename,"file_data":text})),
                },
            })
            .collect::<Result<Vec<_>, CoreError>>()?,
    ))
}

fn encode_media_ref(source: &MediaSource) -> Result<(&'static str, String), CoreError> {
    match source {
        MediaSource::Url { url } => Ok(("image_url", url.clone())),
        MediaSource::File { id: FileId(id) } => Ok(("file_id", id.clone())),
        MediaSource::Data { media_type, bytes } => Ok((
            "image_url",
            format!(
                "data:{};base64,{}",
                media_type.0,
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ),
        )),
    }
}

fn encode_file_ref(source: &MediaSource) -> Result<(&'static str, String), CoreError> {
    match source {
        MediaSource::Url { url } => Ok(("file_url", url.clone())),
        MediaSource::File { id: FileId(id) } => Ok(("file_id", id.clone())),
        MediaSource::Data { media_type, bytes } => Ok((
            "file_data",
            format!(
                "data:{};base64,{}",
                media_type.0,
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ),
        )),
    }
}

fn encode_tool_result(result: &ToolResult) -> Result<Value, CoreError> {
    let output = match &result.outcome {
        ToolOutcome::Success { content } => Value::Array(
            content
                .iter()
                .map(|part| match part {
                    ToolResultContent::Text { text } => {
                        Ok(json!({"type":"input_text","text":text}))
                    }
                    ToolResultContent::Image { source } => {
                        let (field, value) = encode_media_ref(source)?;
                        Ok(json!({"type":"input_image",field:value}))
                    }
                    ToolResultContent::Json { value } => {
                        Ok(json!({"type":"input_text","text":value.to_string()}))
                    }
                })
                .collect::<Result<Vec<_>, CoreError>>()?,
        ),
        ToolOutcome::Error { code, message } => json!({"error":{"code":code,"message":message}}),
    };
    let type_name = match result.kind {
        ToolResultKind::Function => "function_call_output",
        ToolResultKind::Custom => "custom_tool_call_output",
        ToolResultKind::ComputerUse => "computer_call_output",
        ToolResultKind::CodeExecution => "code_interpreter_call_output",
        ToolResultKind::Shell => "shell_call_output",
        ToolResultKind::TextEditor => "apply_patch_call_output",
        ToolResultKind::Memory => "function_call_output",
        ToolResultKind::ToolSearch => "tool_search_output",
    };
    Ok(json!({"type":type_name,"call_id":result.call_id.0,"output":output}))
}

pub(in crate::llm::codec) fn encode_tool(tool: &ToolDefinition) -> Result<Value, CoreError> {
    Ok(match tool {
        ToolDefinition::Function(tool) => {
            json!({"type":"function","name":tool.name,"description":tool.description,"parameters":tool.parameters.0,"strict":tool.strict})
        }
        ToolDefinition::Custom(tool) => {
            json!({"type":"custom","name":tool.name,"description":tool.description,"format":match &tool.input_format { CustomToolInputFormat::Text=>Value::Null, CustomToolInputFormat::Grammar{syntax,definition}=>json!({"type":snake(syntax),"definition":definition}) }})
        }
        ToolDefinition::WebSearch(tool) => {
            let mut value = Map::new();
            value.insert("type".into(), json!("web_search"));
            insert_option(
                &mut value,
                "search_context_size",
                tool.search_context_size.as_ref().map(snake),
            );
            insert_option(&mut value, "user_location", tool.user_location.as_ref());
            insert_option(&mut value, "max_uses", tool.max_uses);
            let mut filters = Map::new();
            if !tool.allowed_domains.is_empty() {
                filters.insert("allowed_domains".into(), json!(tool.allowed_domains));
            }
            if !tool.blocked_domains.is_empty() {
                filters.insert("blocked_domains".into(), json!(tool.blocked_domains));
            }
            if !filters.is_empty() {
                value.insert("filters".into(), Value::Object(filters));
            }
            Value::Object(value)
        }
        ToolDefinition::WebFetch(tool) => {
            json!({"type":"web_fetch","allowed_domains":tool.allowed_domains,"blocked_domains":tool.blocked_domains,"max_uses":tool.max_uses})
        }
        ToolDefinition::FileSearch(tool) => {
            json!({"type":"file_search","vector_store_ids":tool.vector_store_ids,"max_num_results":tool.max_results,"ranking_options":tool.ranking,"filters":tool.filters})
        }
        ToolDefinition::ComputerUse(tool) => {
            json!({"type":"computer_use_preview","environment":snake(&tool.environment),"display_width":tool.display_width,"display_height":tool.display_height})
        }
        ToolDefinition::CodeExecution(tool) => {
            json!({"type":"code_interpreter","container":tool.container.as_deref().unwrap_or("auto")})
        }
        ToolDefinition::Shell(tool) => json!({"type":"shell","environment":tool.environment}),
        ToolDefinition::TextEditor(tool) => {
            json!({"type":"apply_patch","max_characters":tool.max_characters})
        }
        ToolDefinition::ImageGeneration(tool) => {
            json!({"type":"image_generation","size":tool.size,"quality":tool.quality,"background":tool.background,"output_format":tool.output_format})
        }
        ToolDefinition::Mcp(tool) => {
            json!({"type":"mcp","server_label":tool.server_label,"server_url":tool.server_url,"allowed_tools":tool.allowed_tools,"require_approval":encode_mcp_approval(&tool.approval)})
        }
        ToolDefinition::Memory => json!({"type":"memory"}),
        ToolDefinition::ToolSearch(tool) => {
            json!({"type":"tool_search","parameters":{"deferred_tools":tool.deferred_tools}})
        }
    })
}

fn encode_mcp_approval(approval: &McpApproval) -> Value {
    match approval {
        McpApproval::Always => json!("always"),
        McpApproval::Never => json!("never"),
        McpApproval::PerTool { always, never } => {
            json!({"always":{"tool_names":always},"never":{"tool_names":never}})
        }
    }
}

pub(in crate::llm::codec) fn encode_tool_choice(
    choice: &ToolChoice,
    tools: &[ToolDefinition],
) -> Value {
    match choice {
        ToolChoice::Auto => Value::String("auto".into()),
        ToolChoice::None => Value::String("none".into()),
        ToolChoice::Required => Value::String("required".into()),
        ToolChoice::Tool { name } => json!({"type":"function","name":name}),
        ToolChoice::Allowed { names, mode } => {
            json!({"type":"allowed_tools","mode":snake(mode),"tools":names.iter().map(|name|allowed_tool_entry(name, tools)).collect::<Vec<_>>() })
        }
    }
}

/// allowed_tools 条目按定义列表还原工具类型；托管工具按 wire type 名引用。
fn allowed_tool_entry(name: &str, tools: &[ToolDefinition]) -> Value {
    for tool in tools {
        match tool {
            ToolDefinition::Function(tool) if tool.name == name => {
                return json!({"type":"function","name":name})
            }
            ToolDefinition::Custom(tool) if tool.name == name => {
                return json!({"type":"custom","name":name})
            }
            ToolDefinition::Mcp(tool)
                if tool.server_label == name || tool.allowed_tools.iter().any(|n| n == name) =>
            {
                return json!({"type":"mcp","server_label":tool.server_label,"name":name})
            }
            _ => {}
        }
    }
    match name {
        "web_search"
        | "web_fetch"
        | "file_search"
        | "computer_use_preview"
        | "code_interpreter"
        | "shell"
        | "apply_patch"
        | "image_generation"
        | "memory"
        | "tool_search" => json!({"type":name}),
        _ => json!({"type":"function","name":name}),
    }
}

fn encode_output_constraint(output: &OutputConstraint) -> Value {
    match output {
        OutputConstraint::Text => json!({"format":{"type":"text"}}),
        OutputConstraint::JsonObject => json!({"format":{"type":"json_object"}}),
        OutputConstraint::JsonSchema {
            name,
            schema,
            strict,
        } => json!({"format":{"type":"json_schema","name":name,"schema":schema.0,"strict":strict}}),
    }
}

/// OpenAI 系目标是 canonical 直通，gproxy 扩展的工具语义（web_fetch、memory、
/// web_search 限额与屏蔽域名、text_editor 字符上限）没有官方落点，显式
/// route incompatible 参与路由回退，不能静默发给真实上游。
pub(super) fn validate_target_tools(
    request: &GenerateRequest,
    target: OperationKey,
) -> Result<(), CoreError> {
    let OperationKind::ContentGeneration(kind) = target.kind() else {
        return Ok(());
    };
    if !matches!(
        kind,
        ContentGenerationKind::OpenAiResponses
            | ContentGenerationKind::OpenAiResponsesWebSocket
            | ContentGenerationKind::OpenAiChatCompletions
    ) {
        return Ok(());
    }
    let mut fields = Vec::new();
    for tool in &request.tools {
        match tool {
            ToolDefinition::WebFetch(_) => fields.push("tools[].web_fetch".to_owned()),
            ToolDefinition::Memory => fields.push("tools[].memory".to_owned()),
            ToolDefinition::WebSearch(tool) => {
                if tool.max_uses.is_some() {
                    fields.push("tools[].web_search.max_uses".to_owned());
                }
                if !tool.blocked_domains.is_empty() {
                    fields.push("tools[].web_search.blocked_domains".to_owned());
                }
            }
            ToolDefinition::TextEditor(tool) if tool.max_characters.is_some() => {
                fields.push("tools[].text_editor.max_characters".to_owned());
            }
            _ => {}
        }
    }
    if fields.is_empty() {
        Ok(())
    } else {
        Err(CoreError::IncompatibleRoute { target, fields })
    }
}

fn encode_reasoning(reasoning: Option<&ReasoningOptions>) -> Option<Value> {
    let reasoning = reasoning?;
    let mut value = Map::new();
    if let Some(effort) = reasoning.effort {
        value.insert("effort".into(), Value::String(snake(&effort)));
    }
    if let Some(summary) = reasoning.summary {
        value.insert("summary".into(), Value::String(snake(&summary)));
    }
    (!value.is_empty()).then_some(Value::Object(value))
}
