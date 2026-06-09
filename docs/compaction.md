# 上下文压缩策略 (Context Compaction)

本文档详细描述 `agent-tui-full` 的上下文压缩机制，包括三层压缩策略及其配置参数。

---

## 概述

LLM 大语言模型的上下文窗口有限，且上下文越长响应越慢、成本越高。当 agent 执行长时间任务时，对话历史持续增长，必须通过压缩来保持可用的上下文空间。

本系统实现了**三层递进式压缩**：

| 层级 | 触发时机 | 对象 | 策略 |
|------|----------|------|------|
| 第一层：大输出持久化 | 单个工具输出 > 30K 字符 | 单个 tool result | 写磁盘，保留预览 |
| 第二层：微压缩 | 每次 LLM 调用前 | 旧 tool results | 替换为占位符（保留最近 12 条） |
| 第三层：全量压缩 | 上下文 > 500K 字符 | 整个对话历史 | 存档 + LLM 摘要 → 重置上下文 |

---

## 第一层：大输出持久化 (Persist Large Output)

**触发条件**：单个工具调用结果超过 `PERSIST_THRESHOLD`（30,000 字符）。

**处理流程**：
1. 完整输出写入 `.claude/tool-results/{tool_use_id}.txt`
2. 上下文中的 tool result 替换为前 `PREVIEW_CHARS`（2,000 字符）的预览 + 文件路径

**替换格式**：
```xml
<persisted-output>
Full output saved to: .claude/tool-results/abc123.txt
Preview:
[前 2000 字符...]
</persisted-output>
```

### 相关常量

| 常量 | 默认值 | 位置 | 说明 |
|------|--------|------|------|
| `PERSIST_THRESHOLD` | 30,000 | `compact.rs:26` | 触发持久化的字符阈值 |
| `PREVIEW_CHARS` | 2,000 | `compact.rs:28` | 替换文本中保留的预览字符数 |
| `OUTPUT_DIR` | `.claude/tool-results` | `compact.rs:29` | 大输出文件的存储目录 |

---

## 第二层：微压缩 (Micro Compaction)

**触发时机**：每次 agent loop 迭代中，在向 LLM 发送请求之前调用 `micro_compact()`。

**处理流程**：
1. 扫描所有 user 消息中的 `tool_result` 块
2. 保留最近 `KEEP_RECENT_TOOL_RESULTS`（12）条结果不动
3. 对于更早的 tool result，如果其内容超过 120 字符，替换为占位符

**占位符内容**：
```
[Earlier tool result compacted. If you need the full content to continue editing, re-read the relevant file.]
```

**设计意图**：
- 短结果（≤120 字符，如错误信息、确认消息）保留，因为信息密度高、不会占用很多空间
- 长结果被压缩，但 agent 可以通过重新执行工具来获取原始数据
- 最近的 12 条结果保留，保证当前工作流不被打断

### 相关常量

| 常量 | 默认值 | 位置 | 说明 |
|------|--------|------|------|
| `KEEP_RECENT_TOOL_RESULTS` | 12 | `compact.rs:23` | 微压缩中保留的最近 tool result 数量 |
| `COMPACTED_TOOL_RESULT` | 见上 | `compact.rs:31` | 替换旧结果的占位符文本 |

---

## 第三层：全量压缩 (Full Compaction)

**触发时机**：微压缩后，如果序列化上下文仍超过 `context_limit()` 阈值。

### 上下文限制配置

```
默认: 500,000 字符（约 125K tokens）
环境变量覆盖: TACT_CONTEXT_LIMIT_CHARS
```

可以通过设置环境变量调整：
```bash
export TACT_CONTEXT_LIMIT_CHARS=1000000  # 约 250K tokens
```

**处理流程**：

### 步骤 1：保存完整转录 (Transcript)

将完整对话历史序列化为 JSONL 文件，保存至 `.claude/transcripts/transcript_{timestamp}.jsonl`。

### 步骤 2：选取近期消息

从对话历史末端向前遍历，收集最近的若干消息（上限 80,000 字符），**至少保留一条**。这确保了摘要 LLM 能拿到最相关的上下文，而非最早的历史。

### 步骤 3：生成摘要

将选取的消息作为上下文，发送给 LLM（max_tokens=2000），要求保留以下信息：

1. **当前目标和已完成的工作**
2. **重要发现、决策和架构洞察**
3. **读取或修改的文件**（含关键代码结构：类型、签名、API）
4. **剩余工作和下一步**
5. **用户约束和偏好**
6. **遇到的错误及其原因**

如果用户通过 `focus` 参数指定了重点，会在 prompt 中追加。

如果 `recent_files` 不为空（最近 5 个通过 `read_file` 访问的文件），也会注入到 prompt 中。

### 步骤 4：注入最近文件列表

在 LLM 返回的摘要末尾，追加最近访问的文件列表，帮助 agent 在「失忆」后快速恢复上下文：

```
Recently accessed files (re-read if you need their contents):
- src/main.rs
- src/lib.rs
```

### 步骤 5：替换上下文

整个对话历史被替换为一条 user 消息：

```
This conversation was compacted so the agent can continue working.

[LLM 生成的摘要]

Recently accessed files (re-read if you need their contents):
- [文件列表]
```

`compact_state.has_compacted` 标记为 `true`，`last_summary` 保存为当前摘要。

### 相关常量

| 常量 | 默认值 | 位置 | 说明 |
|------|--------|------|------|
| `context_limit()` | 500,000 | `lib.rs:74` | 触发全量压缩的字符数阈值 |
| `TRANSCRIPT_DIR` | `.claude/transcripts` | `compact.rs:29` | 转录文件存储目录 |
| 摘要 prompt 的 max_tokens | 2,000 | `lib.rs:775` | 摘要 LLM 调用的最大 token 数 |
| 近期消息选取上限 | 80,000 字符 | `lib.rs:732` | 选取给摘要 LLM 的上下文大小 |

---

## 最近文件追踪 (Recent Files)

`CompactState.recent_files` 追踪 agent 最近通过 `read_file` 工具访问的文件路径（最多 5 个）。

**更新逻辑**（`remember_recent_file`）：
- 如果文件已存在列表中，先移除旧条目（去重）
- 将文件追加到列表末尾
- 如果超过 5 个，移除最旧的条目（FIFO）

**用途**：
- 在全量压缩的摘要 prompt 中列出，提示 LLM 哪些文件是当前工作重点
- 在最终摘要中注入文件列表，帮助 agent 恢复上下文后快速定位关键文件

---

## 数据流总览

```
Agent Loop 每次迭代
│
├─ micro_compact()                         [第二层]
│   └─ 替换旧 tool results（保留最近 12 条）
│
├─ estimate_context_size() > limit?        [第三层触发检查]
│   ├─ 否 → 继续
│   └─ 是 → compact_history():
│       ├─ write_transcript()              → .claude/transcripts/*.jsonl
│       ├─ 选取最近消息（≤80K chars）
│       ├─ LLM 生成摘要
│       ├─ 注入 recent_files
│       └─ context = compacted_context(summary)
│
└─ LLM 调用
    │
    └─ 工具执行 → 拦截
        ├─ read_file → remember_recent_file(path)
        └─ persist_large_output()          [第一层]
            ├─ 输出 ≤30K chars → 不改动
            └─ 输出 >30K chars → 写磁盘 + 返回预览
```

---

## 与 Agent 系统提示的配合

压缩后的占位符提示 agent：「被压缩的工具结果可以重新运行工具来获取完整内容」。系统提示中包含对应的指导：

```
- If a tool result was compacted and you need the details, re-run the relevant tool (e.g., read_file)
```

这确保了 agent 在遇到被压缩的 tool result 时，可以主动通过 `read_file` 等工具恢复所需数据。
