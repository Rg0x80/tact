// Agent 核心模块
// 负责接收用户任务，调用 OpenAI API 生成执行计划，并在沙箱中逐步执行。
// 通过 Channel 与 TUI 模块通信，实时上报执行状态。

use anyhow::{Result, anyhow};
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestMessage, ChatCompletionRequestUserMessage,
        CreateChatCompletionRequest, Role,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::time::{Duration, sleep, timeout};
use tools::Sandbox;

/// 步骤执行状态。
#[derive(Debug, Clone)]
pub enum StepStatus {
    Success,
    Failed,
}

/// 步骤执行结果的结构化信息。
#[derive(Debug, Clone)]
pub struct StepResult {
    pub tool: String,
    pub arg_summary: String,
    pub status: StepStatus,
    pub message: String,
    /// 附加详情，如文件写入的完整内容或命令输出的原始文本。
    pub detail: Option<String>,
    /// 工具执行耗时（毫秒）。None 表示非工具执行步骤。
    pub duration_ms: Option<u64>,
}

/// 模型调用参数信息。
#[derive(Debug, Clone)]
pub struct ModelCallParams {
    pub model: String,
    pub max_tokens: u32,
    pub thinking_budget: Option<u32>,
    pub reasoning_effort: Option<String>,
    pub extra_body: Option<String>,
}

/// 错误的分类，让 TUI 能够区分需要展示为 ❌ Error 的致命错误
/// 和应作为 Info 提示的非致命情况。
#[derive(Debug, Clone)]
pub enum AgentErrorKind {
    /// 余额查询失败（网络或 API 错误）
    BalanceQueryFailed(String),
    /// 余额查询仅 DeepSeek provider 支持
    BalanceNotSupported,
    /// 通用错误（兜底）
    Other(String),
}

impl AgentErrorKind {
    /// 返回人类可读的错误描述。
    pub fn display(&self) -> &str {
        match self {
            AgentErrorKind::BalanceQueryFailed(e) => e,
            AgentErrorKind::BalanceNotSupported => {
                "Balance query is only available for DeepSeek provider"
            }
            AgentErrorKind::Other(msg) => msg,
        }
    }
}

/// Agent 向 TUI 推送的状态更新消息。
#[derive(Debug)]
pub enum AgentUpdate {
    /// 计划已生成，附带步骤列表
    PlanGenerated(Vec<PlanStep>),
    /// 第 idx 步开始执行
    StepStarted(usize),
    /// 第 idx 步执行成功，附带结构化结果
    StepFinished(usize, StepResult),
    /// 第 idx 步执行失败，附带错误信息
    StepFailed(usize, String),
    /// 需要用户审批：提示文本、步骤索引、审批通道（true=同意，false=拒绝）
    NeedApproval(String, usize, oneshot::Sender<bool>),
    /// 整个任务完成
    TaskComplete(String),
    /// Agent 错误，携带分类便于 TUI 决定展示方式
    Error(AgentErrorKind),
    /// Token 消耗统计
    TokenUsage {
        prompt: u32,
        completion: u32,
        total: u32,
        /// DeepSeek KV cache 命中的 prompt token 数（非 DeepSeek provider 时为 0）
        prompt_cache_hit_tokens: u32,
        /// DeepSeek KV cache 未命中的 prompt token 数
        prompt_cache_miss_tokens: u32,
    },
    /// 账户余额信息（仅 DeepSeek）
    Balance(BalanceInfo),
    /// 模型调用参数（名称、max_tokens、thinking budget 等）
    ModelInfo(ModelCallParams),
    /// 纯信息提示（不改变状态）
    Info(String),
    /// 动态追加一个步骤到已有计划中（不重置选择状态）
    StepAdded(PlanStep),
    /// 请求用户从选项列表中选择一个，返回选项索引（None 表示取消）
    RequestSelect {
        prompt: String,
        options: Vec<String>,
        respond: oneshot::Sender<Option<usize>>,
    },
    /// 流式输出文本片段（实时追加到 Log）
    StreamChunk(String),
    /// 流式 thinking / reasoning 内容片段
    ThinkingChunk(String),
}

/// TUI 向 Agent 发送的用户命令。
#[derive(Debug)]
pub enum UserCommand {
    /// 提交一个新的自然语言任务
    SubmitTask(String),
    /// 取消当前任务（暂未实现完整取消逻辑）
    Cancel,
    /// 查询账户余额（仅 DeepSeek）
    QueryBalance,
}

/// 执行计划中的单个步骤。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    /// 步骤描述（人类可读）
    pub description: String,
    /// 工具名称：read_file / write_file / run_command
    pub tool: String,
    /// 工具参数（键值对）
    pub args: HashMap<String, String>,
    /// 执行前是否需要用户手动审批
    pub need_approval: bool,
    /// 执行后的输出（由 TUI 填充，JSON 反序列化时默认为 None）
    #[serde(default)]
    pub output: Option<String>,
}

/// DeepSeek 账户余额信息的单个币种条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceEntry {
    /// 货币类型：CNY 或 USD
    pub currency: String,
    /// 总可用余额（赠金 + 充值）
    pub total_balance: String,
    /// 未过期赠金余额
    pub granted_balance: String,
    /// 充值余额
    pub topped_up_balance: String,
}

/// DeepSeek 账户余额查询结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceInfo {
    /// 账户是否有可用余额
    pub is_available: bool,
    /// 各币种余额明细
    pub balance_infos: Vec<BalanceEntry>,
}

/// 智能体结构体，持有沙箱、OpenAI 客户端及通信通道。
pub struct Agent {
    sandbox: Arc<Sandbox>,
    openai_client: Client<OpenAIConfig>,
    /// 向 TUI 发送状态更新的通道
    ui_tx: UnboundedSender<AgentUpdate>,
    /// 接收 TUI 发来的用户命令
    cmd_rx: UnboundedReceiver<UserCommand>,
    /// 任务取消标志，TUI 设置后 Agent 在执行间隙检查并提前退出
    cancel_flag: Arc<AtomicBool>,
}

impl Agent {
    pub fn new(
        ui_tx: UnboundedSender<AgentUpdate>,
        cmd_rx: UnboundedReceiver<UserCommand>,
    ) -> Self {
        // 以当前目录为工作空间，构建沙箱
        let workspace = PathBuf::from(".");
        // 允许执行的命令白名单
        let allowed_commands = vec![
            "cargo".to_string(),
            "git".to_string(),
            "python".to_string(),
            "npm".to_string(),
        ];
        let sandbox = Sandbox::new(workspace, allowed_commands);
        let openai_client = Client::new();
        Self {
            sandbox: Arc::new(sandbox),
            openai_client,
            ui_tx,
            cmd_rx,
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Agent 主循环：持续监听用户命令，直到通道关闭。
    pub async fn run(mut self) -> Result<()> {
        while let Some(cmd) = self.cmd_rx.recv().await {
            match cmd {
                UserCommand::SubmitTask(task) => {
                    if let Err(e) = self.handle_task(task).await {
                        let _ = self
                            .ui_tx
                            .send(AgentUpdate::Error(AgentErrorKind::Other(e.to_string())));
                    }
                }
                UserCommand::Cancel => {
                    self.cancel_flag.store(true, Ordering::Relaxed);
                    let _ = self
                        .ui_tx
                        .send(AgentUpdate::Info("Cancelling current task...".into()));
                }
                UserCommand::QueryBalance => {
                    let _ = self
                        .ui_tx
                        .send(AgentUpdate::Error(AgentErrorKind::BalanceNotSupported));
                }
            }
        }
        Ok(())
    }

    /// 处理单个任务：生成计划 -> 逐歩执行 -> 上报结果。
    async fn handle_task(&self, task: String) -> Result<()> {
        self.cancel_flag.store(false, Ordering::Relaxed);

        // 1. 调用 LLM 生成执行计划
        let plan = self.generate_plan(&task).await?;
        if self.cancel_flag.load(Ordering::Relaxed) {
            self.ui_tx.send(AgentUpdate::StepFailed(
                0,
                "Cancelled by user before execution".into(),
            ))?;
            return Ok(());
        }
        self.ui_tx.send(AgentUpdate::PlanGenerated(plan.clone()))?;

        // 2. 按顺序执行每一步
        for (idx, step) in plan.iter().enumerate() {
            if self.cancel_flag.load(Ordering::Relaxed) {
                self.ui_tx
                    .send(AgentUpdate::StepFailed(idx, "Cancelled by user".into()))?;
                return Ok(());
            }
            self.ui_tx.send(AgentUpdate::StepStarted(idx))?;

            // 若步骤标记为需审批，则通过 oneshot channel 等待 TUI 的用户确认
            if step.need_approval {
                let (tx, mut rx) = oneshot::channel();
                self.ui_tx
                    .send(AgentUpdate::NeedApproval(step.description.clone(), idx, tx))?;
                // 每 100ms 轮询一次，兼顾取消响应与 CPU 占用
                let approved = loop {
                    if self.cancel_flag.load(Ordering::Relaxed) {
                        self.ui_tx.send(AgentUpdate::StepFailed(
                            idx,
                            "Cancelled by user during approval".into(),
                        ))?;
                        return Ok(());
                    }
                    match rx.try_recv() {
                        Ok(result) => break result,
                        Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                            sleep(Duration::from_millis(100)).await;
                            continue;
                        }
                        Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                            return Err(anyhow!("User approval cancelled"));
                        }
                    }
                };
                if !approved {
                    self.ui_tx
                        .send(AgentUpdate::StepFailed(idx, "Rejected by user".into()))?;
                    return Ok(());
                }
            }

            // 在沙箱中执行工具调用
            let result = self.execute_step(&step).await;
            match result {
                Ok(output) => {
                    let arg_summary = match step.tool.as_str() {
                        "read_file" | "write_file" => {
                            step.args.get("path").cloned().unwrap_or_default()
                        }
                        "run_command" => step.args.get("command").cloned().unwrap_or_default(),
                        _ => step.args.values().next().cloned().unwrap_or_default(),
                    };
                    let preview = output.chars().take(200).collect::<String>();
                    let detail = match step.tool.as_str() {
                        "write_file" => step.args.get("content").cloned(),
                        "run_command" => Some(output),
                        _ => None,
                    };
                    let step_result = StepResult {
                        tool: step.tool.clone(),
                        arg_summary,
                        status: StepStatus::Success,
                        message: preview,
                        detail,
                        duration_ms: None,
                    };
                    self.ui_tx
                        .send(AgentUpdate::StepFinished(idx, step_result))?;
                }
                Err(e) => {
                    self.ui_tx
                        .send(AgentUpdate::StepFailed(idx, e.to_string()))?;
                    return Ok(());
                }
            }
            // 每步之间短暂停顿，便于 TUI 展示动画效果
            sleep(Duration::from_millis(200)).await;
        }

        self.ui_tx.send(AgentUpdate::TaskComplete(
            "All steps completed successfully!".into(),
        ))?;
        Ok(())
    }

    /// 调用 OpenAI ChatCompletion API，要求模型返回固定格式的 JSON 计划数组。
    async fn generate_plan(&self, task: &str) -> Result<Vec<PlanStep>> {
        let model = "gpt-3.5-turbo".to_string();
        let _ = self.ui_tx.send(AgentUpdate::ModelInfo(ModelCallParams {
            model: model.clone(),
            max_tokens: 0,
            thinking_budget: None,
            reasoning_effort: None,
            extra_body: None,
        }));
        let json_request = CreateChatCompletionRequest {
            model,
            messages: vec![ChatCompletionRequestMessage::User(
                ChatCompletionRequestUserMessage {
                    content: format!(
                        "You must output ONLY a valid JSON array of steps. Each step has fields: description (str), tool (one of read_file, write_file, run_command), args (object with keys appropriate for the tool), need_approval (bool).\n\nTask: {}",
                        task
                    ).into(),
                    role: Role::User,
                    name: None,
                },
            )],
            temperature: Some(0.2),
            ..Default::default()
        };
        let _ = self
            .ui_tx
            .send(AgentUpdate::Info("Calling LLM API...".into()));
        let resp = timeout(
            Duration::from_secs(30),
            self.openai_client.chat().create(json_request),
        )
        .await
        .map_err(|_| anyhow!("OpenAI API timed out after 30s"))??;
        let _ = self.ui_tx.send(AgentUpdate::StepFinished(
            0,
            StepResult {
                tool: "generate_plan".to_string(),
                arg_summary: String::new(),
                status: StepStatus::Success,
                message: "API response received".to_string(),
                detail: None,
                duration_ms: None,
            },
        ));
        if let Some(usage) = resp.usage {
            let _ = self.ui_tx.send(AgentUpdate::TokenUsage {
                prompt: usage.prompt_tokens,
                completion: usage.completion_tokens,
                total: usage.total_tokens,
                prompt_cache_hit_tokens: 0,
                prompt_cache_miss_tokens: 0,
            });
        }
        let content = resp
            .choices
            .get(0)
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default();
        // 清理 LLM 可能包裹的 markdown 代码块
        let cleaned = content
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();
        let plan: Vec<PlanStep> = serde_json::from_str(cleaned)?;
        Ok(plan)
    }

    /// 根据步骤中指定的工具，在沙箱中执行具体操作。
    async fn execute_step(&self, step: &PlanStep) -> Result<String> {
        match step.tool.as_str() {
            "read_file" => {
                let path = step.args.get("path").ok_or(anyhow!("Missing path"))?;
                let content = self.sandbox.read_file(path).await?;
                Ok(format!("Read {} bytes", content.len()))
            }
            "write_file" => {
                let path = step.args.get("path").ok_or(anyhow!("Missing path"))?;
                let content = step.args.get("content").ok_or(anyhow!("Missing content"))?;
                self.sandbox.write_file(path, content).await?;
                Ok(format!("Written to {}", path))
            }
            "run_command" => {
                let command = step.args.get("command").ok_or(anyhow!("Missing command"))?;
                let default_args = "[]".to_string();
                let args_str = step.args.get("args").unwrap_or(&default_args);
                let args: Vec<&str> = serde_json::from_str(args_str)?;
                let output = self.sandbox.run_command(command, &args).await?;
                Ok(format!("Command output:\n{}", output))
            }
            _ => Err(anyhow!("Unknown tool: {}", step.tool)),
        }
    }
}
