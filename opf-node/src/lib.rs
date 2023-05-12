use std::sync::mpsc::Sender;

use log::info;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use opf_models::error::ErrorKind;
use opf_models::event::{send_event, Domain, Event};
use opf_models::{CommandAction, CommandObject};

mod help;
mod parse;

#[derive(Debug)]
pub struct Node {
    pub cli_tx: Sender<Event>,
    pub self_tx: UnboundedSender<(Domain, Event)>,
    pub self_rx: UnboundedReceiver<(Domain, Event)>,
    pub module_tx: UnboundedSender<Event>,
    pub db_tx: UnboundedSender<Event>,
}

impl Node {
    pub async fn new(cli_tx: Sender<Event>) -> (UnboundedSender<(Domain, Event)>, Self) {
        let (self_tx, self_rx) = tokio::sync::mpsc::unbounded_channel::<(Domain, Event)>();
        let (db_tx, database_controller) = opf_db::new(self_tx.clone()).await;
        let (module_tx, module_controller) = opf_modules::new(self_tx.clone());

        tokio::spawn(database_controller.launch());
        tokio::spawn(module_controller.launch());

        let tx = self_tx.clone();
        (
            tx,
            Node {
                cli_tx,
                self_tx,
                self_rx,
                db_tx,
                module_tx,
            },
        )
    }

    pub async fn main_loop(mut self) {
        info!("running opf node...");
        loop {
            tokio::select! {
                Some((domain, event)) = self.self_rx.recv() => {
                    let _ = match domain {
                        Domain::Data => send_event(&self.db_tx, event).await,
                        Domain::Node => {
                             match event {
                               Event::NewCommand(command) => self.on_command(command).await,
                                _ => Ok(())
                            }
                        },
                        Domain::Module => {
                            match event {
                                Event::ExecuteModule(_) => send_event(&self.module_tx, event).await,
                                Event::ListModules => send_event(&self.module_tx, event).await,
                                _ => Ok(())
                            }
                        },
                        Domain::Network => Ok(()),
                        Domain::CLI => {
                            self.cli_tx.send(event)
                                 .map_err(|e| ErrorKind::Channel(e.to_string()))
                        }
                    };
                }
            }
        }
    }

    async fn on_command(&mut self, cmd: String) -> Result<(), ErrorKind> {
        match parse::format(cmd.as_str()) {
            Ok(command) => match command.object {
                CommandObject::Module(ref module_name) => match &command.action {
                    CommandAction::Help => {
                        send_event(&self.module_tx, Event::HelpModule(module_name.to_string()))
                            .await
                    }
                    CommandAction::List => send_event(&self.module_tx, Event::ListModules).await,
                    CommandAction::Run => {
                        send_event(
                            &self.db_tx,
                            Event::PrepareModule((module_name.clone(), command)),
                        )
                        .await
                    }
                    _ => Ok(()),
                },
                CommandObject::Export(_) => {
                    send_event(&self.db_tx, Event::CommandExport(command)).await
                }
                CommandObject::Target => {
                    send_event(&self.db_tx, Event::CommandTarget(command)).await
                }
                CommandObject::Group => send_event(&self.db_tx, Event::CommandGroup(command)).await,
                CommandObject::Workspace => {
                    send_event(&self.db_tx, Event::CommandWorkspace(command)).await
                }
                CommandObject::Keystore => {
                    send_event(&self.db_tx, Event::CommandKeystore(command)).await
                }
                CommandObject::Link => send_event(&self.db_tx, Event::CommandLink(command)).await,
                CommandObject::None => match command.action {
                    CommandAction::Help => self.on_help().await,
                    _ => Ok(()),
                },
                _ => Ok(()),
            },
            Err(e) => self
                .cli_tx
                .send(Event::ResponseError(e.to_string()))
                .map_err(|e| ErrorKind::Channel(e.to_string())),
        }
    }
}
