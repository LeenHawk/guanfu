# SillyTavern 预设、角色卡、世界书、正则与资源格式调查

## 1. 调查范围

本文面向观复阶段五的实体与导入导出设计，结论来自仓库内的
`samples/SillyTavern`，而不是对所有历史版本或第三方分支的概括。

- SillyTavern 版本：`1.18.0`
- 样本提交：`380e31e8c58d196969b6a0da74f431ba999c7e0a`
- 提交日期：`2026-07-12`
- 主要依据：
  - `src/types/spec-v2.d.ts`
  - `src/validator/TavernCardValidator.js`
  - `src/endpoints/characters.js`
  - `src/character-card-parser.js`
  - `src/charx.js`
  - `src/endpoints/presets.js`
  - `public/scripts/PromptManager.js`
  - `public/scripts/preset-manager.js`
  - `public/scripts/char-data.js`
  - `public/scripts/world-info.js`
  - `public/scripts/extensions/regex/engine.js`
  - `public/scripts/extensions/regex/index.js`
  - `public/scripts/personas.js`

样本中的代码存在比声明文件更宽的兼容行为。下文将“角色卡标准字段”、
“SillyTavern 扩展字段”和“仅供 SillyTavern 本地运行的状态”分开描述。

## 2. 总体关系

```text
生成预设 ──包含──> 采样参数、模型/连接选择、提示词与提示词顺序
   │                         │
   └── extensions.regex_scripts ──> 预设级正则

角色卡 ──包含──> 角色文本、开场白、提示词覆盖、标签
   ├── data.character_book ──> 可交换的嵌入世界书
   ├── data.extensions.regex_scripts ──> 角色级正则
   ├── data.assets / CharX ──> 头像、表情、背景等资源
   └── data.extensions.world ──> SillyTavern 本地“主世界书”名称

独立世界书 <──绑定── 全局 / 角色主世界书 / 角色附加世界书 / 会话 / persona
```

这里最重要的边界是：嵌入世界书和世界书绑定不是一回事。前者随角色卡
交换，后者大多是 SillyTavern 本地设置或会话状态。

## 3. 预设（presets）

### 3.1 文件和类别

预设均为单个 JSON 文件，文件名（去掉 `.json`）是预设名称。后端根据
`apiId` 把它们存入不同目录：

| `apiId` | 默认内容目录 | 用途 |
| --- | --- | --- |
| `openai` | `presets/openai` | Chat Completion 连接、生成参数和提示词编排 |
| `textgenerationwebui` | `presets/textgen` | Text Completion 采样参数 |
| `kobold` / `koboldhorde` | `presets/kobold` | Kobold 采样参数 |
| `novel` | `presets/novel` | NovelAI 采样参数 |
| `instruct` | `presets/instruct` | 指令模板 |
| `context` | `presets/context` | story string / 上下文模板 |
| `sysprompt` | `presets/sysprompt` | 系统提示词模板 |
| `reasoning` | `presets/reasoning` | 思维内容的前后缀与分隔符 |

同为“预设”的 JSON 并不存在一个统一 schema。导入判断也是启发式的，例如：

- instruct：至少含 `name`、`input_sequence`、`output_sequence`
- context：至少含 `name`、`story_string`
- sysprompt：至少含 `name`、`content`
- text completion：至少含 `temp`、`top_k`、`top_p`、`rep_pen`
- reasoning：至少含 `name`、`prefix`、`suffix`、`separator`

因此兼容层应保留预设类别，不能只建一个不带类型的“参数集合”。

### 3.2 OpenAI/Chat Completion 预设

`default/content/presets/openai/Default.json` 展示了最完整的格式，字段可分为：

- 连接与模型：`chat_completion_source`、各 provider 的 `*_model`、
  `custom_url`、`reverse_proxy` 等。
- 采样：`temperature`、`frequency_penalty`、`presence_penalty`、`top_p`、
  `top_k`、`top_a`、`min_p`、`repetition_penalty`、`seed`、`n`。
- 长度与流式：`openai_max_context`、`openai_max_tokens`、`stream_openai`。
- 提示词辅助：`send_if_empty`、`impersonation_prompt`、`new_chat_prompt`、
  `new_group_chat_prompt`、`new_example_chat_prompt`、`continue_nudge_prompt`、
  `group_nudge_prompt`、`wi_format`、`scenario_format`、`personality_format`。
- 编排：`prompts` 和 `prompt_order`。
- 扩展：`extensions`，正则扩展使用 `extensions.regex_scripts`。

其中 proxy/custom endpoint 字段会在导入和导出时被视为敏感项，包括
`reverse_proxy`、`proxy_password`、`custom_url`、`custom_include_body`、
`custom_exclude_body`、`custom_include_headers` 等。它们不应进入观复的普通、
可分享预设实体；连接配置应由 channel/credential 负责。

#### `prompts[]`

提示词项由 `PromptManager` 定义，常见字段如下：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `identifier` | string | 稳定标识；排序和覆盖按它关联 |
| `name` | string | 展示名 |
| `role` | string | 通常为 `system` / `user` / `assistant` |
| `content` | string | 提示词正文；marker 项可没有正文 |
| `system_prompt` | boolean | 是否为系统内置提示词 |
| `marker` | boolean | 是否是角色描述、历史、世界书等动态槽位 |
| `injection_position` | number | `0` 相对编排，`1` 按聊天深度注入 |
| `injection_depth` | number | 深度注入位置 |
| `injection_order` | number | 同深度下的次序，默认 100 |
| `injection_trigger` | string[] | 限定生成类型 |
| `forbid_overrides` | boolean | 禁止角色卡覆盖此提示词 |
| `extension` | boolean | 是否由扩展提供 |

内置 marker 包括 `worldInfoBefore`、`worldInfoAfter`、`charDescription`、
`charPersonality`、`scenario`、`personaDescription`、`dialogueExamples` 和
`chatHistory`。

#### `prompt_order[]`

每项形如：

```json
{
  "character_id": 100000,
  "order": [
    { "identifier": "main", "enabled": true },
    { "identifier": "worldInfoBefore", "enabled": true }
  ]
}
```

`character_id` 在默认预设中也用于虚拟顺序：`100000` 是无 persona 的默认
顺序，`100001` 是含 persona 的默认顺序。因此它不宜直接映射成观复角色表
外键；导入时应把它理解为一个 prompt-order profile。

### 3.3 其他预设的主要字段

- Text Completion：大量 sampler 字段，例如 `temp`、`top_*`、`min_p`、
  `rep_pen*`、`dry_*`、`mirostat_*`、`sampler_order`、`grammar_string`、
  `json_schema`、`logit_bias`。字段会随后端能力变化。
- Context：`story_string`、`chat_start`、`example_separator`、
  `story_string_position`、`story_string_role`、`story_string_depth`、停止字符串
  和名称处理选项。
- Instruct：user/assistant/system 的首个、普通、最后一个 sequence 与 suffix，
  `stop_sequence`、`wrap`、`macro`、名称策略及 `activation_regex`。
- System Prompt：`name`、`content`、`post_history`。
- Reasoning：`name`、`prefix`、`suffix`、`separator`。运行时 settings 还有
  `auto_parse`、`add_to_prompts` 等状态，但默认 preset 文件不保存这些状态。

兼容策略上，生成参数适合按已知字段结构化；provider 专用和未来新增字段应
保留在带版本的扩展 JSON 中，避免导入后丢失。

## 4. 角色卡（character card）

### 4.1 容器格式

SillyTavern 当前支持导入 `json`、`png`、`charx`、`yaml/yml` 和 `byaf`，
原生导出只提供 JSON 与 PNG。

- JSON：角色卡对象本身。
- PNG：在 PNG `tEXt` chunk 中放 base64 JSON。
  - `chara`：CCv2 数据。
  - `ccv3`：CCv3 数据；读取时优先于 `chara`。
  - 写出 PNG 时会同时写 `chara` 和一个把 `spec`/`spec_version` 改为 V3 的
    `ccv3` chunk。
- CharX：ZIP；根目录必须有 `card.json`，并可包含 `data.assets` 指向的文件。

默认 `default_Seraphina.png` 同时包含 `chara` 和 `ccv3`。两份数据的字段
实际上相同，仅 spec 标识不同。

### 4.2 V1 兼容层

V1 只要求顶层包含：

`name`、`description`、`personality`、`scenario`、`first_mes`、`mes_example`。

SillyTavern 还会在顶层维护 `creatorcomment`、`talkativeness`、`fav`、`tags`、
`avatar`、`chat`、`create_date` 等旧字段或本地字段。对于 V2 卡，同一份核心
内容常被重复 hoist 到顶层以兼容旧代码。导入观复时应以 `data.*` 为准，顶层
副本只用于兼容和冲突诊断。

### 4.3 CCv2

外层固定字段：

```json
{
  "spec": "chara_card_v2",
  "spec_version": "2.0",
  "data": {}
}
```

`data` 的必需字段：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `name` | string | 角色名 |
| `description` | string | 角色描述 |
| `personality` | string | 性格描述 |
| `scenario` | string | 场景设定 |
| `first_mes` | string | 首条开场消息 |
| `mes_example` | string | 示例对话 |
| `creator_notes` | string | 作者说明 |
| `system_prompt` | string | 主系统提示词覆盖 |
| `post_history_instructions` | string | 历史后指令覆盖 |
| `alternate_greetings` | string[] | 其他开场白 |
| `tags` | string[] | 标签 |
| `creator` | string | 作者 |
| `character_version` | string | 角色卡版本 |
| `extensions` | object | 开放扩展命名空间 |

可选的 `character_book` 是嵌入世界书，见下一节。样本角色还含非 V2 声明
字段 `group_only_greetings`，说明兼容导入不能采用拒绝未知字段的模型。

### 4.4 SillyTavern 角色扩展

样本明确使用以下 `data.extensions`：

```json
{
  "talkativeness": 0.5,
  "fav": false,
  "world": "Eldoria",
  "depth_prompt": {
    "prompt": "...",
    "depth": 4,
    "role": "system"
  },
  "regex_scripts": []
}
```

- `fav` 和 `chat` 属于私有/本地状态；导出时会清除 `fav` 和当前聊天名。
- `world` 是对本地独立世界书的名称引用，不等同于 `character_book`。
- `depth_prompt` 是角色专用的按深度提示词。
- `regex_scripts` 是角色作用域的正则脚本。
- 外部工具还可能放入 `risuai`、`chub`、`pygmalion_id`、`source_url`、
  `sd_character_prompt` 等扩展，SillyTavern 会尽量透传未知扩展。

### 4.5 CCv3 的实际支持边界

当前 validator 只检查：

- `spec == "chara_card_v3"`
- `3.0 <= Number(spec_version) < 4.0`
- `data` 是对象

它没有在样本仓库内声明或验证完整 CCv3 字段。因此不能从这份 sample 推导
完整的 CCv3 schema。观复若要声称完整 CCv3 兼容，还需单独依据正式规范实现；
这里能确认的只是 SillyTavern 对 V3 容器的宽松接收和 CharX 资源读取行为。

## 5. 世界书（World Info / Lorebook）

### 5.1 两种同时存在的数据形态

#### 可交换的 Character Book

角色卡中的 `data.character_book` 采用数组：

```json
{
  "name": "Eldoria",
  "description": "",
  "scan_depth": 4,
  "token_budget": 2048,
  "recursive_scanning": false,
  "extensions": {},
  "entries": []
}
```

其中只有 `entries` 和 `extensions` 被当前 V2 validator 强制要求；其他字段均
可选。现实卡片可能省略顶层 `extensions`，但样本 validator 会拒绝这种卡。

Character Book entry 的标准/兼容字段为：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `keys` | string[] | 主触发词/正则 |
| `secondary_keys` | string[] | 次触发词 |
| `content` | string | 注入正文 |
| `enabled` | boolean | 是否启用 |
| `insertion_order` | number | 优先/插入顺序 |
| `case_sensitive` | boolean? | 是否大小写敏感 |
| `name` / `comment` | string? | 展示名/备注 |
| `id` | number? | entry 标识；没有时 ST 用数组下标补齐 |
| `priority` | number? | 兼容优先级字段 |
| `selective` | boolean? | 是否使用次关键词条件 |
| `constant` | boolean? | 常驻激活 |
| `position` | string? | `before_char` / `after_char` |
| `extensions` | object | SillyTavern 和其他工具扩展 |

SillyTavern 写出的 entry 还会带 `use_regex: true`。

#### SillyTavern 独立世界书

独立世界书是一个 JSON 文件，核心形态为：

```json
{
  "entries": {
    "0": {
      "uid": 0,
      "key": [],
      "keysecondary": [],
      "content": "..."
    }
  }
}
```

`entries` 是以 uid 字符串为 key 的对象，而不是数组。SillyTavern 只硬性检查
顶层存在 `entries`，读取列表时还会保留可选顶层 `name` 和 `extensions`。

独立 entry 的完整当前模板字段为：

- 基本：`uid`、`key`、`keysecondary`、`comment`、`content`、`order`、
  `displayIndex`、`disable`。
- 激活：`constant`、`vectorized`、`selective`、`selectiveLogic`、
  `probability`、`useProbability`。
- 插入：`position`、`depth`、`role`、`outletName`、`ignoreBudget`。
- 递归：`excludeRecursion`、`preventRecursion`、`delayUntilRecursion`、
  `scanDepth`。
- 匹配：`caseSensitive`、`matchWholeWords`、`matchPersonaDescription`、
  `matchCharacterDescription`、`matchCharacterPersonality`、
  `matchCharacterDepthPrompt`、`matchScenario`、`matchCreatorNotes`。
- 分组/时效：`group`、`groupOverride`、`groupWeight`、`useGroupScoring`、
  `sticky`、`cooldown`、`delay`。
- 自动化：`automationId`、`triggers`。

枚举值：

- `selectiveLogic`：`0 AND_ANY`、`1 NOT_ALL`、`2 NOT_ANY`、`3 AND_ALL`。
- `position`：`0 before`、`1 after`、`2 ANTop`、`3 ANBottom`、
  `4 atDepth`、`5 EMTop`、`6 EMBottom`、`7 outlet`。
- `role`：`0 system`、`1 user`、`2 assistant`。
- `triggers` 已知值：`normal`、`continue`、`impersonate`、`swipe`、
  `regenerate`、`quiet`。

### 5.2 两种形态的转换和保真

`convertCharacterBook()` 把数组 entry 映射为 ST 内部 entry，并把原对象保存在
`originalData`。编辑世界书时，代码会同步回写 `originalData`，角色再次导出时
优先使用它。这是为了避免一次导入导出破坏原 Character Book 的未知字段。

映射的主要差异包括：

- `keys` ↔ `key`
- `secondary_keys` ↔ `keysecondary`
- `insertion_order` ↔ `order`
- `enabled` ↔ `!disable`
- `extensions.position` ↔ `position`
- 其余 ST 高级字段多数位于 Character Book entry 的 `extensions` 中，采用
  snake_case；内部运行时则多为 camelCase。

因此观复应把世界书和 entry 正规化为实体，同时保存未识别扩展/原始交换
信息。只保存 SillyTavern 内部模板会损失外部格式，反之只保存 Character Book
也不能表达 ST 的全部运行语义。

### 5.3 世界书的五种绑定作用域

| 作用域 | SillyTavern 存储位置 |
| --- | --- |
| 全局启用 | settings 中 `world_info.globalSelect` / `selected_world_info` |
| 角色主世界书 | 角色卡 `data.extensions.world`，按世界书名称引用 |
| 角色附加世界书 | settings 中 `world_info.charLore[] = { name: 角色文件基名, extraBooks: [] }` |
| 会话世界书 | `chat_metadata.world_info` |
| persona 世界书 | persona descriptor 的 `lorebook`，并有当前 persona 的兼容设置字段 |

生成时会合并上述来源并去重。角色附加世界书绑定不随角色卡导出；会话和
persona 绑定也不是 Character Book 的一部分。观复不应在 `character` 表上只放
一个 `world_book_id`，而应使用带 owner/scope/order 的关联实体。

## 6. 正则脚本

### 6.1 脚本结构

SillyTavern 的 Regex 扩展定义如下：

```json
{
  "id": "uuid",
  "scriptName": "display name",
  "findRegex": "/pattern/gi",
  "replaceString": "$1",
  "trimStrings": [],
  "placement": [1, 2],
  "disabled": false,
  "markdownOnly": false,
  "promptOnly": false,
  "runOnEdit": false,
  "substituteRegex": 0,
  "minDepth": null,
  "maxDepth": null
}
```

| 字段 | 说明 |
| --- | --- |
| `id` | UUID；旧脚本没有时会迁移生成 |
| `scriptName` | 展示名 |
| `findRegex` | JS 风格 `/pattern/flags` 字符串 |
| `replaceString` | 支持 `{{match}}`、`$1`、`$<name>`，最终还会做 macro 替换 |
| `trimStrings` | 在捕获内容写入 replacement 前，从捕获内容移除的字符串 |
| `placement` | 生效阶段数组 |
| `disabled` | 禁用脚本 |
| `markdownOnly` | 只用于展示格式化 |
| `promptOnly` | 只用于发给模型的内容 |
| `runOnEdit` | 编辑消息时也执行 |
| `substituteRegex` | find regex 中的宏处理方式 |
| `minDepth` / `maxDepth` | 只处理指定聊天深度范围 |

`placement` 当前枚举：`1 USER_INPUT`、`2 AI_OUTPUT`、`3 SLASH_COMMAND`、
`5 WORLD_INFO`、`6 REASONING`。`0 MD_DISPLAY` 已废弃，会迁移为新的 only
标志；旧值 `4 sendAs` 会迁移到 `SLASH_COMMAND`。

`substituteRegex`：`0 NONE`、`1 RAW`、`2 ESCAPED`。

### 6.2 三种作用域和执行顺序

| 作用域 | 存储位置 | 是否需要用户授权 |
| --- | --- | --- |
| 全局 | `extension_settings.regex` | 扩展启用即可 |
| 角色 | `character.data.extensions.regex_scripts` | 角色 avatar 必须在 `character_allowed_regex` |
| 预设 | `preset.extensions.regex_scripts` | API + preset 名必须在 `preset_allowed_regex` |

合并执行的固定顺序由 `SCRIPT_TYPES` 的值决定：全局（0）→ 角色（1）→
预设（2）。数组内顺序也有语义，不能用无序集合保存。

角色卡或预设携带正则不代表自动执行；授权是接收方本地的信任状态，不应随
共享资源一起导入为已授权。正则可修改 prompt 和显示内容，应视为主动内容，
导入后默认禁用或等待显式授权。

## 7. 相关资源

### 7.1 角色头像和 CharX assets

PNG 角色卡同时是默认头像。CharX 中 `card.json` 的 `data.assets[]` 至少按
以下字段读取：

```json
{
  "type": "icon",
  "name": "main",
  "uri": "embedded://assets/main.png",
  "ext": "png"
}
```

- `type`：已明确处理 `icon`、`user_icon`、`emotion`、`expression`、
  `background`，其他类型归入 misc。
- `name`：同类资源的逻辑名称；主头像优先选择 `type=icon,name=main`，否则
  使用第一个 icon。
- `uri`：sample 的 CharX 导入器只提取嵌入 URI，兼容
  `embedded://`、RisuAI 的误拼 `embeded://` 和 `__asset:`。
- `ext`：扩展名；没有时从 ZIP path 推导。

当前导入器只落盘图片扩展名：`png`、`jpg`、`jpeg`、`webp`、`gif`、
`apng`、`avif`、`bmp`、`jfif`。

落盘映射为：

- `icon`：角色主 PNG/头像，不作为 auxiliary asset 重复保存。
- `emotion` / `expression`：角色 sprite 目录。
- `background`：角色专属 `backgrounds` 目录。
- 其他图片：角色 image gallery/misc 目录。

这些路径是 SillyTavern 的文件系统实现细节。观复更适合使用统一 `asset` 实体
保存 blob/object key、MIME、大小和校验值，再用带 `kind`、`name`、`order` 的
关系连接角色、persona、世界书或消息。

### 7.2 SillyTavern 本地 asset library

独立 asset API 的分类为：`bgm`、`ambient`、`blip`、`live2d`、`vrm`、
`character`、`temp`。此外还有：

- 全局背景图。
- persona avatar（文件名是 persona 的本地标识）。
- 角色 sprites/expressions。
- 角色专属背景和 gallery 图片。
- 聊天附件与生成图片，它们不属于角色卡标准。

资源分类会继续扩展，不宜把 MIME 或文件扩展名编码成业务 enum；`kind` 可用
可扩展枚举/字符串，实际媒体类型使用 MIME。

### 7.3 Persona 的相关定义

虽然 persona 不属于角色卡标准，但阶段五需要兼容其提示词和资源关联。当前
SillyTavern persona 以 avatar 文件名为 key，名称在 `personas[avatarId]`，描述
对象在 `persona_descriptions[avatarId]`：

```json
{
  "description": "...",
  "position": 0,
  "depth": 2,
  "role": 0,
  "lorebook": "book name",
  "title": "..."
}
```

`position`：`0 IN_PROMPT`、`1 AFTER_CHAR`（已废弃）、`2 TOP_AN`、
`3 BOTTOM_AN`、`4 AT_DEPTH`、`9 NONE`；`role` 同样是 0/1/2。

persona 还可被锁定到 chat、character/group 或设为默认。观复应以稳定 ID
建 persona，而不是沿用 avatar 文件名作为主键；头像是可替换资源。

## 8. 对观复阶段五实体设计的直接建议

本节只给出从兼容调查能直接推出的最小边界，不展开完整 schema：

1. `character` 保存可编辑的核心角色字段；开场白拆成有序子项，不能只留一个
   `first_message`。
2. `persona` 是独立实体，头像使用 asset 关系；默认/会话锁定属于选择或绑定
   状态，而不是 persona 文本本身。
3. `world_book` 与 `world_book_entry` 独立建模；角色嵌入只是导入导出形态。
4. 使用有序的 `world_book_binding` 表达 global、character primary、character
   additional、conversation、persona 等 scope。
5. `regex_script` 需要有序绑定到 global、character 或 preset；信任/授权单独
   保存，导入内容不能携带授权结果。
6. preset 至少带明确 kind。通用的生成参数、prompt item、prompt order 可以
   结构化；provider/扩展未知字段需要保真存储。
7. `asset` 与业务对象解耦；角色头像、persona 头像、expression、background、
   gallery、音频等通过关系记录 kind/name/order。
8. 对 `extensions` 和未识别字段保留 JSON；但核心查询、排序、外键和运行语义
   不能只塞 JSON。导入时记录来源格式/spec version，导出时由 adapter 重建。
9. SillyTavern 使用文件名或名称做多处引用，观复内部应使用稳定 ID；导出到
   SillyTavern 时再生成名称引用并处理冲突。

## 9. 兼容验收样本

后续实现最小可用导入导出时，可直接使用 sample 内这些 fixture：

- 角色卡：`default/content/default_Seraphina.png`
- 独立世界书：`default/content/Eldoria.json`
- Chat Completion 预设：`default/content/presets/openai/Default.json`
- Text Completion 预设：`default/content/presets/textgen/Default.json`
- Context：`default/content/presets/context/Default.json`
- Instruct：`default/content/presets/instruct/ChatML.json`
- System Prompt：`default/content/presets/sysprompt/Roleplay - Simple.json`
- Reasoning：`default/content/presets/reasoning/Think XML.json`

建议的最小 round-trip 验收不是 JSON 字节完全相同，而是：标准字段、未知
extensions、entry 顺序、正则顺序、资源关系和世界书绑定语义均不丢失。
