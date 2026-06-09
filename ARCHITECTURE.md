# 架构与流程文档

本文档通过 Mermaid 图表描述 `agent-tui-full` 的整体架构、核心数据流及终端界面布局。

---

## 0. Workspace 结构

本项目是一个 Cargo Workspace，包含以下 crate：

| 目录 | 包名 | 职责 |
|---|---|---|
| `crates/core` | `tact_core` | 共享类型：`AgentUpdate`、`UserCommand`、`PlanStep`、`StepResult`、`StepStatus` |
| `crates/tools` | `tools` | `Sandbox`：文件读写、命令执行的安全封装 |
| `crates/tui` | `tui` | 基于 `ratatui` 的终端交互界面 |
| `crates/tact` | `tact` | Agent runtime、主循环、工具路由、CLI 入口 |
| `crates/tool_refactor_macros` | `tool_refactor_macros` | 工具重构相关的过程宏 |

依赖关系（简图）：

```mermaid
flowchart TB
    tact --> tact_core
    tact --> tui
    tact --> tool_refactor_macros
    tui --> tact_core
    tact_core --> tools
```

---

## 1. 模块架构图

```mermaid
flowchart TB
    subgraph main["main.rs"]
        M["main()<br/>初始化运行时、通道、启动任务"]
    end

    subgraph agent_mod["tact/src/lib.rs — Agent 核心"]
        A["Agent 结构体"]
        AG["generate_plan()<br/>调用 OpenAI API"]
        AE["execute_step()<br/>调用沙箱工具"]
        A --> AG
        A --> AE
    end

    subgraph tools_mod["tools crate — 沙箱工具"]
        S["Sandbox"]
        SR["read_file()"]
        SW["write_file()"]
        SC["run_command()"]
        S --> SR
        S --> SW
        S --> SC
    end

    subgraph tui_mod["tui/ — 终端界面"]
        T["mod.rs<br/>事件循环"]
        TH["handlers.rs<br/>按键处理"]
        TR["render.rs<br/>绘制面板"]
        TS["state.rs<br/>App 状态"]
        TT["theme.rs<br/>主题配色"]
        T --> TH
        T --> TR
        T --> TS
        TR --> TS
        TH --> TS
        TS --> TT
    end

    M -- "mpsc 通道" --> A
    M -- "mpsc 通道" --> T
    A -- "Arc<Sandbox>" --> S
    AE -- "工具调用" --> S
    T -- "UnboundedSender" --> A
    A -- "AgentUpdate" --> T
```

---

## 2. Agent 任务执行流程图

```mermaid
sequenceDiagram
    actor U as 用户
    participant TUI as TUI 模块
    participant Agent as Agent 模块
    participant LLM as OpenAI API
    participant SB as Sandbox

    U ->> TUI: 输入任务并回车
    TUI ->> Agent: UserCommand::SubmitTask
    Agent ->> LLM: generate_plan(task)
    LLM -->> Agent: JSON 计划数组
    Agent ->> TUI: AgentUpdate::PlanGenerated

    loop 逐歩执行
        Agent ->> TUI: AgentUpdate::StepStarted(idx)
        alt need_approval = true
            Agent ->> TUI: AgentUpdate::NeedApproval
            TUI ->> U: 显示审批提示 (y/n)
            U -->> TUI: y / n
            TUI -->> Agent: oneshot::Sender<bool>
            alt 用户拒绝
                Agent ->> TUI: AgentUpdate::StepFailed
                Note over Agent,TUI: 终止任务
            end
        end
        Agent ->> SB: execute_step(step)
        SB -->> Agent: 结果 / 错误
        alt 执行成功
            Agent ->> TUI: AgentUpdate::StepFinished
        else 执行失败
            Agent ->> TUI: AgentUpdate::StepFailed
            Note over Agent,TUI: 终止任务
        end
    end

    Agent ->> TUI: AgentUpdate::TaskComplete
    TUI ->> U: 显示完成消息
```

---

## 3. TUI 渲染布局图

```mermaid
block-beta
    columns 1
    space
    block:status
        columns 1
        status_bar["Status Bar (高度 1)"]
    end
    block:main
        columns 2
        plan["Plan Panel<br/>(40% 宽度)<br/>执行计划列表<br/>▼ 展开 / ▶ 折叠"]
        log["Log Panel<br/>(60% 宽度)<br/>消息滚动区域<br/>支持搜索高亮"]
    end
    block:input
        columns 1
        input_box["Input Box (高度 3)<br/>Insert 模式: 任务输入<br/>Command 模式: :cmd<br/>Search 模式: /term"]
    end
    space

    style status_bar fill:#2e3440,color:#eceff4
    style plan fill:#2e3440,color:#eceff4
    style log fill:#2e3440,color:#eceff4
    style input_box fill:#2e3440,color:#eceff4
```

### 覆盖层（弹出面板）

```mermaid
block-beta
    columns 1
    space
    block:overlay
        columns 1
        help["Help Panel<br/>键盘快捷键一览"]
        history["History Panel<br/>任务历史记录"]
        palette["Command Palette<br/>过滤命令列表"]
    end
    space

    style help fill:#1e1e28,color:#eceff4
    style history fill:#1e1e28,color:#eceff4
    style palette fill:#1e1e28,color:#eceff4
```

---

## 4. 事件循环流程图

```mermaid
flowchart TD
    Start([启动 TUI]) --> Init["enable_raw_mode<br/>EnterAlternateScreen"]
    Init --> InitApp["初始化 App 状态"]
    InitApp --> LoopStart{主循环}

    LoopStart --> Draw["terminal.draw()<br/>渲染所有面板"]
    Draw --> PollAgent["try_recv()<br/>消费 Agent 更新"]
    PollAgent --> PollEvent["event::poll(50ms)<br/>检测终端事件"]

    PollEvent -- "无事件" --> CheckQuit{should_quit?}
    PollEvent -- "有事件" --> HandleEvent["处理 Key / Mouse / Resize"]

    HandleEvent --> KeyCheck{按键类型?}
    KeyCheck -- "Ctrl+C" --> SetQuit["should_quit = true"]
    KeyCheck -- "Ctrl+H" --> ToggleHist["toggle show_history"]
    KeyCheck -- "Ctrl+T" --> ToggleTheme["toggle_theme()"]
    KeyCheck -- "Ctrl+?" --> ToggleHelp["toggle show_help"]
    KeyCheck -- "普通按键" --> ModeDispatch["按 input_mode 分发"]

    ModeDispatch --> Normal["handle_normal_mode()"]
    ModeDispatch --> Insert["handle_insert_mode()"]
    ModeDispatch --> Command["handle_command_mode()"]
    ModeDispatch --> Search["handle_search_mode()"]
    ModeDispatch --> Palette["handle_palette_mode()"]

    HandleEvent --> Mouse["Mouse 事件:<br/>滚轮滚动 / 拖拽选择"]
    HandleEvent --> Resize["Resize 事件:<br/>重新计算布局"]

    SetQuit --> CheckQuit
    ToggleHist --> CheckQuit
    ToggleTheme --> CheckQuit
    ToggleHelp --> CheckQuit
    Normal --> CheckQuit
    Insert --> CheckQuit
    Command --> CheckQuit
    Search --> CheckQuit
    Palette --> CheckQuit
    Mouse --> CheckQuit
    Resize --> CheckQuit

    CheckQuit -- "否" --> LoopStart
    CheckQuit -- "是" --> Cleanup["disable_raw_mode<br/>LeaveAlternateScreen"]
    Cleanup --> End([退出])
```

---

## 5. 通道通信架构图

```mermaid
flowchart LR
    subgraph Channels["Tokio MPSC 通道"]
        direction LR
        TX1["ui_tx<br/>(UnboundedSender<AgentUpdate>)"]
        RX1["agent_rx<br/>(UnboundedReceiver<AgentUpdate>)"]
        TX2["user_cmd_tx<br/>(UnboundedSender<UserCommand>)"]
        RX2["cmd_rx<br/>(UnboundedReceiver<UserCommand>)"]
    end

    subgraph AgentTask["Agent 异步任务"]
        A["Agent"]
    end

    subgraph MainThread["主线程"]
        TUI["TUI 事件循环"]
    end

    A -- "发送状态更新" --> TX1
    TX1 -- "AgentUpdate" --> RX1
    RX1 --> TUI

    TUI -- "发送用户命令" --> TX2
    TX2 -- "UserCommand" --> RX2
    RX2 --> A

    style TX1 fill:#bf616a,color:#eceff4
    style RX1 fill:#bf616a,color:#eceff4
    style TX2 fill:#a3be8c,color:#2e3440
    style RX2 fill:#a3be8c,color:#2e3440
```

---

## 6. 沙箱安全路径处理流程

```mermaid
flowchart TD
    Input["safe_path(relative_path)"] --> Filter["过滤路径组件:<br/>- 保留 Normal<br/>- 弹出 ParentDir(..)<br/>- 忽略 RootDir / Prefix"]
    Filter --> Join["拼接 workspace_root"]
    Join --> Exist{"文件/目录<br/>是否存在?"}

    Exist -- "存在" --> Canonical["canonicalize()<br/>解析符号链接"]
    Exist -- "不存在" --> ParentExist{"父目录<br/>是否存在?"}

    ParentExist -- "存在" --> ParentCano["parent.canonicalize()<br/>+ file_name"]
    ParentExist -- "不存在" --> PrefixCheck{"starts_with<br/>workspace_root?"}

    PrefixCheck -- "否" --> Err1["返回错误:<br/>Path escapes workspace"]
    PrefixCheck -- "是" --> Return1["返回 full 路径"]

    Canonical --> Check{"starts_with<br/>canonical_root?"}
    ParentCano --> Check

    Check -- "否" --> Err2["返回错误:<br/>Path escapes workspace"]
    Check -- "是" --> Return2["返回 safe PathBuf"]

    Err1 --> End([结束])
    Err2 --> End
    Return1 --> End
    Return2 --> End
```
