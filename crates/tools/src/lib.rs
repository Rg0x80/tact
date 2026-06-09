// 沙箱工具模块
// 提供受限的文件读写和命令执行能力，防止 Agent 操作逃逸出指定工作目录。

use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::process::Command;

/// 沙箱结构体，限制所有文件与命令操作在 `workspace_root` 范围内。
pub struct Sandbox {
    /// 工作空间根目录（原始路径）
    workspace_root: PathBuf,
    /// 规范化后的绝对路径（用于最终校验）
    canonical_root: PathBuf,
    /// 允许执行的命令白名单
    allowed_commands: Vec<String>,
}

impl Sandbox {
    pub fn new(workspace_root: PathBuf, allowed_commands: Vec<String>) -> Self {
        // canonicalize 解析符号链接，得到真实的绝对路径
        let canonical_root = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.clone());
        Self {
            workspace_root,
            canonical_root,
            allowed_commands,
        }
    }

    /// 路径安全检查：
    /// 1. 过滤 `..` 和根目录/前缀组件，防止目录遍历攻击
    /// 2. 解析符号链接，防止通过软链接逃逸沙箱
    /// 3. 最终校验规范化后的路径是否仍在 `canonical_root` 之内
    fn safe_path(&self, relative_path: &str) -> Result<PathBuf> {
        let path = Path::new(relative_path);
        let mut components = Vec::new();
        for comp in path.components() {
            match comp {
                std::path::Component::Normal(c) => components.push(c),
                std::path::Component::ParentDir => {
                    components.pop();
                }
                _ => {}
            }
        }
        let cleaned = components.iter().collect::<PathBuf>();
        let full = self.workspace_root.join(&cleaned);

        // 解析符号链接以防止沙箱逃逸
        let resolved = if full.exists() {
            full.canonicalize()?
        } else if let Some(parent) = full.parent() {
            if parent.exists() {
                parent
                    .canonicalize()?
                    .join(full.file_name().unwrap_or_default())
            } else {
                // 父目录链不存在 —— 退回到前缀检查
                if !full.starts_with(&self.workspace_root) {
                    return Err(anyhow!("Path escapes workspace: {}", relative_path));
                }
                return Ok(full);
            }
        } else {
            return Err(anyhow!("Invalid path: {}", relative_path));
        };

        if !resolved.starts_with(&self.canonical_root) {
            return Err(anyhow!("Path escapes workspace: {}", relative_path));
        }
        Ok(full)
    }

    /// 安全读取文件内容。
    pub async fn read_file(&self, path: &str) -> Result<String> {
        let full = self.safe_path(path)?;
        let content = fs::read_to_string(&full).await?;
        Ok(content)
    }

    /// 安全写入文件。若文件已存在则自动创建 `.bak` 备份；自动创建缺失的父目录。
    pub async fn write_file(&self, path: &str, content: &str) -> Result<()> {
        let full = self.safe_path(path)?;
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).await?;
        }
        if full.exists() {
            let backup = full.with_extension("bak");
            fs::copy(&full, &backup).await?;
        }
        fs::write(&full, content).await?;
        Ok(())
    }

    /// 在沙箱工作目录下执行命令。仅允许白名单中的命令，返回 stdout 或 stderr 错误。
    pub async fn run_command(&self, cmd: &str, args: &[&str]) -> Result<String> {
        let base_cmd = cmd.split_whitespace().next().unwrap_or(cmd);
        if !self.allowed_commands.contains(&base_cmd.to_string()) {
            return Err(anyhow!("Command not allowed: {}", base_cmd));
        }

        let output = Command::new(cmd)
            .args(args)
            .current_dir(&self.workspace_root)
            .output()
            .await?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(anyhow!("Command failed: {}", stderr))
        }
    }
}
