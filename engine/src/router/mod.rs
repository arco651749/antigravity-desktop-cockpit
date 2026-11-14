//! WebSocket 指令分派路由模組，依功能領域拆分為視窗調度、輸入控制與 UIA 語意操作。

mod input_ops;
mod uia_ops;
mod window_ops;

use crate::protocol::ClientCommand;
use crate::server::StreamServer;
use tokio::sync::mpsc;

impl StreamServer {
    pub async fn dispatch_command(
        &self,
        cmd: ClientCommand,
        ctrl_tx: &mpsc::Sender<String>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match cmd {
            ClientCommand::GetUIATree { .. }
            | ClientCommand::UIAInvoke { .. }
            | ClientCommand::UIAAction { .. }
            | ClientCommand::GetVisualDiff => self.handle_uia_command(cmd, ctrl_tx).await,

            ClientCommand::RestorePrimaryFocus
            | ClientCommand::Mouse { .. }
            | ClientCommand::Key { .. }
            | ClientCommand::TypeText { .. }
            | ClientCommand::GetClipboard
            | ClientCommand::SetClipboard { .. } => self.handle_input_command(cmd, ctrl_tx).await,

            _ => self.handle_window_command(cmd, ctrl_tx).await,
        }
    }
}
