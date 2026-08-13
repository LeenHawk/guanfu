use sea_orm::entity::prelude::*;

use gproxy_protocol::{ContentGenerationKind, Operation, OperationKind, Provider};

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    EnumIter,
    DeriveActiveEnum,
    serde::Deserialize,
    serde::Serialize,
    ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::N(32))")]
pub enum RoutingOperation {
    #[sea_orm(string_value = "list_models")]
    ListModels,
    #[sea_orm(string_value = "get_model")]
    GetModel,
    #[sea_orm(string_value = "count_tokens")]
    CountTokens,
    #[sea_orm(string_value = "generate_content")]
    GenerateContent,
    #[sea_orm(string_value = "stream_generate_content")]
    StreamGenerateContent,
    #[sea_orm(string_value = "create_image")]
    CreateImage,
    #[sea_orm(string_value = "edit_image")]
    EditImage,
    #[sea_orm(string_value = "web_search")]
    WebSearch,
    #[sea_orm(string_value = "rerank")]
    Rerank,
    #[sea_orm(string_value = "create_embedding")]
    CreateEmbedding,
    #[sea_orm(string_value = "create_speech")]
    CreateSpeech,
    #[sea_orm(string_value = "create_transcription")]
    CreateTranscription,
    #[sea_orm(string_value = "create_translation")]
    CreateTranslation,
    #[sea_orm(string_value = "compact_content")]
    CompactContent,
    #[sea_orm(string_value = "create_conversation")]
    CreateConversation,
    #[sea_orm(string_value = "create_realtime_call")]
    CreateRealtimeCall,
    #[sea_orm(string_value = "connect_realtime")]
    ConnectRealtime,
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    EnumIter,
    DeriveActiveEnum,
    serde::Deserialize,
    serde::Serialize,
    ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::N(32))")]
pub enum RoutingKind {
    #[sea_orm(string_value = "open_ai_responses")]
    OpenAiResponses,
    #[sea_orm(string_value = "open_ai_responses_websocket")]
    #[serde(rename = "open_ai_responses_websocket")]
    OpenAiResponsesWebSocket,
    #[sea_orm(string_value = "open_ai_chat_completions")]
    OpenAiChatCompletions,
    #[sea_orm(string_value = "claude_messages")]
    ClaudeMessages,
    #[sea_orm(string_value = "gemini_generate_content")]
    GeminiGenerateContent,
    #[sea_orm(string_value = "open_ai")]
    OpenAi,
    #[sea_orm(string_value = "claude")]
    Claude,
    #[sea_orm(string_value = "gemini")]
    Gemini,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::N(16))")]
pub enum RouteImplementation {
    #[sea_orm(string_value = "passthrough")]
    Passthrough,
    #[sea_orm(string_value = "transform_to")]
    TransformTo,
    #[sea_orm(string_value = "local")]
    Local,
    #[sea_orm(string_value = "unsupported")]
    Unsupported,
}

/// 渠道路由矩阵中的一个单元格。
///
/// `(channel_id, operation, kind)` 唯一；缺少单元格即不支持。
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "routing_rule")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique_key = "channel_operation_kind")]
    pub channel_id: i32,
    #[sea_orm(unique_key = "channel_operation_kind")]
    pub operation: RoutingOperation,
    #[sea_orm(unique_key = "channel_operation_kind")]
    pub kind: RoutingKind,
    pub implementation: RouteImplementation,
    pub dest_operation: Option<RoutingOperation>,
    pub dest_kind: Option<RoutingKind>,
    pub sort_order: i32,
    pub enabled: bool,
    pub created_at: TimeDateTimeWithTimeZone,
    pub updated_at: TimeDateTimeWithTimeZone,
    #[sea_orm(belongs_to, from = "channel_id", to = "id")]
    pub channel: HasOne<super::channel::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}

impl TryFrom<Operation> for RoutingOperation {
    type Error = ();

    fn try_from(value: Operation) -> Result<Self, Self::Error> {
        Ok(match value {
            Operation::ListModels => Self::ListModels,
            Operation::GetModel => Self::GetModel,
            Operation::CountTokens => Self::CountTokens,
            Operation::GenerateContent => Self::GenerateContent,
            Operation::StreamGenerateContent => Self::StreamGenerateContent,
            Operation::CreateImage => Self::CreateImage,
            Operation::EditImage => Self::EditImage,
            Operation::WebSearch => Self::WebSearch,
            Operation::Rerank => Self::Rerank,
            Operation::CreateEmbedding => Self::CreateEmbedding,
            Operation::CreateSpeech => Self::CreateSpeech,
            Operation::CreateTranscription => Self::CreateTranscription,
            Operation::CreateTranslation => Self::CreateTranslation,
            Operation::CompactContent => Self::CompactContent,
            Operation::CreateConversation => Self::CreateConversation,
            Operation::CreateRealtimeCall => Self::CreateRealtimeCall,
            Operation::ConnectRealtime => Self::ConnectRealtime,
            _ => return Err(()),
        })
    }
}

impl From<RoutingOperation> for Operation {
    fn from(value: RoutingOperation) -> Self {
        match value {
            RoutingOperation::ListModels => Self::ListModels,
            RoutingOperation::GetModel => Self::GetModel,
            RoutingOperation::CountTokens => Self::CountTokens,
            RoutingOperation::GenerateContent => Self::GenerateContent,
            RoutingOperation::StreamGenerateContent => Self::StreamGenerateContent,
            RoutingOperation::CreateImage => Self::CreateImage,
            RoutingOperation::EditImage => Self::EditImage,
            RoutingOperation::WebSearch => Self::WebSearch,
            RoutingOperation::Rerank => Self::Rerank,
            RoutingOperation::CreateEmbedding => Self::CreateEmbedding,
            RoutingOperation::CreateSpeech => Self::CreateSpeech,
            RoutingOperation::CreateTranscription => Self::CreateTranscription,
            RoutingOperation::CreateTranslation => Self::CreateTranslation,
            RoutingOperation::CompactContent => Self::CompactContent,
            RoutingOperation::CreateConversation => Self::CreateConversation,
            RoutingOperation::CreateRealtimeCall => Self::CreateRealtimeCall,
            RoutingOperation::ConnectRealtime => Self::ConnectRealtime,
        }
    }
}

impl TryFrom<OperationKind> for RoutingKind {
    type Error = ();

    fn try_from(value: OperationKind) -> Result<Self, Self::Error> {
        Ok(match value {
            OperationKind::ContentGeneration(ContentGenerationKind::OpenAiResponses) => {
                Self::OpenAiResponses
            }
            OperationKind::ContentGeneration(ContentGenerationKind::OpenAiResponsesWebSocket) => {
                Self::OpenAiResponsesWebSocket
            }
            OperationKind::ContentGeneration(ContentGenerationKind::OpenAiChatCompletions) => {
                Self::OpenAiChatCompletions
            }
            OperationKind::ContentGeneration(ContentGenerationKind::ClaudeMessages) => {
                Self::ClaudeMessages
            }
            OperationKind::ContentGeneration(ContentGenerationKind::GeminiGenerateContent) => {
                Self::GeminiGenerateContent
            }
            OperationKind::Provider(Provider::OpenAi) => Self::OpenAi,
            OperationKind::Provider(Provider::Claude) => Self::Claude,
            OperationKind::Provider(Provider::Gemini) => Self::Gemini,
            _ => return Err(()),
        })
    }
}

impl From<RoutingKind> for OperationKind {
    fn from(value: RoutingKind) -> Self {
        match value {
            RoutingKind::OpenAiResponses => {
                Self::ContentGeneration(ContentGenerationKind::OpenAiResponses)
            }
            RoutingKind::OpenAiResponsesWebSocket => {
                Self::ContentGeneration(ContentGenerationKind::OpenAiResponsesWebSocket)
            }
            RoutingKind::OpenAiChatCompletions => {
                Self::ContentGeneration(ContentGenerationKind::OpenAiChatCompletions)
            }
            RoutingKind::ClaudeMessages => {
                Self::ContentGeneration(ContentGenerationKind::ClaudeMessages)
            }
            RoutingKind::GeminiGenerateContent => {
                Self::ContentGeneration(ContentGenerationKind::GeminiGenerateContent)
            }
            RoutingKind::OpenAi => Self::Provider(Provider::OpenAi),
            RoutingKind::Claude => Self::Provider(Provider::Claude),
            RoutingKind::Gemini => Self::Provider(Provider::Gemini),
        }
    }
}
