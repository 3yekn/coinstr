// Copyright (c) 2022-2023 Coinstr
// Distributed under the MIT software license

use std::str::FromStr;

use coinstr_sdk::core::bdk::miniscript::Descriptor;
use coinstr_sdk::core::bips::bip32::Fingerprint;
use coinstr_sdk::core::signer::{Signer, SignerType};
use iced::widget::{Column, Row, Space};
use iced::{Alignment, Command, Element, Length};

use crate::app::component::Dashboard;
use crate::app::{Context, Message, Stage, State};
use crate::component::{button, Text, TextInput};
use crate::constants::APP_NAME;
use crate::theme::color::DARK_RED;

#[derive(Debug, Clone)]
pub enum AddSignerMessage {
    NameChanged(String),
    FingerprintChanged(String),
    DescriptorChanged(String),
    ErrorChanged(Option<String>),
    SaveSigner,
}

#[derive(Debug, Default)]
pub struct AddSignerState {
    name: String,
    fingerprint: String,
    descriptor: String,
    error: Option<String>,
}

impl AddSignerState {
    pub fn new() -> Self {
        Self::default()
    }
}

impl State for AddSignerState {
    fn title(&self) -> String {
        format!("{APP_NAME} - Add signer")
    }

    fn update(&mut self, ctx: &mut Context, message: Message) -> Command<Message> {
        if let Message::AddSigner(msg) = message {
            match msg {
                AddSignerMessage::NameChanged(name) => self.name = name,
                AddSignerMessage::FingerprintChanged(fingerprint) => self.fingerprint = fingerprint,
                AddSignerMessage::DescriptorChanged(desc) => self.descriptor = desc,
                AddSignerMessage::ErrorChanged(error) => self.error = error,
                AddSignerMessage::SaveSigner => {
                    let client = ctx.client.clone();
                    let name = self.name.clone();
                    let fingerprint = self.fingerprint.clone();
                    let descriptor = self.descriptor.clone();
                    return Command::perform(
                        async move {
                            let fingerprint = Fingerprint::from_str(&fingerprint)?;
                            let descriptor = Descriptor::from_str(&descriptor)?;
                            let signer = Signer::new(
                                name,
                                None,
                                fingerprint,
                                descriptor,
                                SignerType::AirGap,
                            )?;
                            client.save_signer(signer).await?;
                            Ok::<(), Box<dyn std::error::Error>>(())
                        },
                        |res| match res {
                            Ok(_) => Message::View(Stage::Signers),
                            Err(e) => AddSignerMessage::ErrorChanged(Some(e.to_string())).into(),
                        },
                    );
                }
            }
        }

        Command::none()
    }

    fn view(&self, ctx: &Context) -> Element<Message> {
        let name = TextInput::new("Name", &self.name)
            .on_input(|s| AddSignerMessage::NameChanged(s).into())
            .placeholder("Name")
            .view();

        let fingerprint = TextInput::new("Fingerprint", &self.fingerprint)
            .on_input(|s| AddSignerMessage::FingerprintChanged(s).into())
            .placeholder("Master fingerprint")
            .view();

        let descriptor = TextInput::new("Descriptor", &self.descriptor)
            .on_input(|s| AddSignerMessage::DescriptorChanged(s).into())
            .placeholder("Descriptor")
            .view();

        let error = if let Some(error) = &self.error {
            Row::new().push(Text::new(error).color(DARK_RED).view())
        } else {
            Row::new()
        };

        let save_signer_btn = button::primary("Save signer")
            .on_press(AddSignerMessage::SaveSigner.into())
            .width(Length::Fill);

        let content = Column::new()
            .push(
                Column::new()
                    .push(Text::new("Create signer").size(24).bold().view())
                    .push(
                        Text::new("Create a new airgapped signer")
                            .extra_light()
                            .view(),
                    )
                    .spacing(10)
                    .width(Length::Fill),
            )
            .push(name)
            .push(fingerprint)
            .push(descriptor)
            .push(error)
            .push(Space::with_height(Length::Fixed(15.0)))
            .push(save_signer_btn)
            .align_items(Alignment::Center)
            .spacing(10)
            .padding(20)
            .max_width(400);

        Dashboard::new().view(ctx, content, true, true)
    }
}

impl From<AddSignerState> for Box<dyn State> {
    fn from(s: AddSignerState) -> Box<dyn State> {
        Box::new(s)
    }
}

impl From<AddSignerMessage> for Message {
    fn from(msg: AddSignerMessage) -> Self {
        Self::AddSigner(msg)
    }
}
