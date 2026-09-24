//! Run a turn on `codex app-server`, steer it while it runs, and print the answer.
//!
//! The prompt makes the model run a shell command that takes a few seconds,
//! so there is time to add input before the turn ends. Needs an authenticated
//! `codex` on `PATH`.
//!
//! ```sh
//! cargo run --example app_server --features app-server
//! ```

use std::time::Duration;

use codex_wrapper::app_server::{
    AppServer, ServerMessage, ThreadStartParams, TurnStartParams, UserInput,
};
use codex_wrapper::{ApprovalPolicy, Codex, SandboxMode};

#[tokio::main]
async fn main() -> codex_wrapper::Result<()> {
    let codex = Codex::builder().build()?;
    let mut server = AppServer::builder(&codex).start().await?;
    println!("server: {}", server.server_info().user_agent);

    let thread = server
        .thread_start(
            ThreadStartParams::new()
                .ephemeral(true)
                .approval_policy(ApprovalPolicy::Never)
                .sandbox(SandboxMode::ReadOnly),
        )
        .await?;
    let turn = server
        .turn_start(TurnStartParams::text(
            &thread.id,
            "Run the shell command `sleep 6`, then reply with the single word DONE.",
        ))
        .await?;

    // A handle can be moved to another task. Here it sends one steer after a
    // few seconds, while this task reads events.
    let handle = server.handle();
    let (thread_id, turn_id) = (thread.id.clone(), turn.id.clone());
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let steered = handle
            .turn_steer(
                &thread_id,
                &turn_id,
                vec![UserInput::text("Also append the word STEERED after DONE.")],
            )
            .await;
        println!("steer: {steered:?}");
    });

    let mut answer = None;
    while let Some(message) = server.next_message().await? {
        match message {
            ServerMessage::Notification(notification) => {
                if let Some(message) = notification.agent_message()
                    && message.is_final_answer()
                {
                    answer = Some(message.text);
                }
                if let Some(done) = notification.turn_completed() {
                    println!(
                        "turn {:?}: {}",
                        done.turn.status,
                        answer.unwrap_or_default()
                    );
                    break;
                }
            }
            // With this approval policy the server should not ask. An
            // unanswered request would stall the turn, so refuse it.
            ServerMessage::Request(request) => {
                server.respond_error(request.id, -32601, "not supported")?;
            }
            _ => {}
        }
    }

    server.shutdown().await
}
