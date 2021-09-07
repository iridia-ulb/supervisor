use shared::UpMessage;
use web_sys::{Element, HtmlInputElement};
use yew::{prelude::*, services::ConsoleService, web_sys::HtmlTextAreaElement};

use yew::services::reader::{File, FileChunk, FileData, ReaderService, ReaderTask};
use yew::{html, ChangeData, Component, ComponentLink, Html, ShouldRender};

use shared::experiment::Request;

//use wasm_bindgen::prelude::*;

use crate::UserInterface as Parent;

pub struct ConfigCard {
    link: ComponentLink<Self>,
    props: Props,
    tasks: Vec<ReaderTask>,
    software_checksums: Vec<(String, String)>,
    software_status: Result<(), String>,
}

// what if properties was just drone::Instance itself?
#[derive(Clone, Properties)]
pub struct Props {
    pub parent: ComponentLink<Parent>,
}

pub enum Msg {
    UpdateSoftware(Vec<(String, String)>, Result<(), String>),

    // this msg probably doesn't belong here
    // since it does interact with this component
    AddSoftware(Vec<File>),
    ResetSoftware,
}

// is it possible to just add a callback to the update method
impl Component for ConfigCard {
    type Message = Msg;
    type Properties = Props;

    fn create(props: Props, link: ComponentLink<Self>) -> Self {

        props.parent.send_message(crate::Msg::SetDroneConfigComp(link.clone()));
        // if props contains a closure, I could use that to communicate with the actual instance
        ConfigCard { 
            props,
            link,
            tasks: Default::default(),
            software_checksums: Default::default(),
            software_status: Err(String::from("No software uploaded"))
        }
    }

    // this fires when a message needs to be processed
    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        //let mut drone = self.props.instance.borrow_mut();
        match msg {
            Msg::ResetSoftware => todo!(),
            // TODO move this code into the other callback and 
            // remove this message type from this component
            Msg::AddSoftware(files) => for file in files {
                let upload = self.props.parent.callback(|FileData {name, content}| {
                    let up_msg = UpMessage::Experiment(Request::AddDroneSoftware(name, content));
                    crate::Msg::SendUpMessage(up_msg)
                });
                self.tasks.push(ReaderService::read_file(file, upload).unwrap());
            },
            Msg::UpdateSoftware(checksums, status) => {
                self.software_checksums = checksums;
                self.software_status = status;
            }
        }
        true
    }

    // this fires when the parent changes the properties of this component
    fn change(&mut self, _: Self::Properties) -> ShouldRender {
        true
    }

    // `self.link.callback(...)` can only be created with a struct that impl Component
    // `|_: ClickEvent| { Msg::Click }` can probably be stored anywhere, i.e., external to the component
    // 
    fn view(&self) -> Html {
        html! {
            <div class="card">
                <header class="card-header">
                    <nav class="card-header-title is-shadowless has-background-white-ter level is-mobile">
                        <div class="level-left">
                            <p class="level-item subtitle is-size-4">{ "Drone Configuration" }</p>
                        </div>
                    </nav>
                </header>
                <div class="card-content">
                    <div class="content">
                        { self.render_config() }
                    </div>
                </div>
                { self.render_menu() }
            </div>
        }
    }
}

impl ConfigCard {
    fn render_config(&self) -> Html {
        html! {
            <>
                <nav class="level is-mobile">
                    <div class="level-left">
                        <p class="level-item">{ "Control software" }</p>
                    </div>
                    <div class="level-right"> {
                        match &self.software_status {
                            Ok(_) => html! {
                                <span class="level-item">
                                    <span class="icon is-medium">
                                        <i class="mdi mdi-24px mdi-check has-text-success"/>
                                    </span>
                                </span>
                            },
                            Err(error) => html! {
                                <span class="level-item">
                                    { error }
                                    <span class="icon is-medium">
                                        <i class="mdi mdi-24px mdi-close has-text-danger" />
                                    </span>
                                </span>
                            }
                        }
                    } </div>
                </nav>
                
                <table class="table is-bordered is-hoverable">
                    <thead>
                        <tr>
                            <th>{ "File" }</th>
                            <th>{ "Checksum" }</th>
                        </tr>
                    </thead>
                    <tbody> {
                        self.software_checksums.iter()
                            .map(|(name, checksum)| html! {
                                <tr>
                                    <td> { name } </td>
                                    <td> { checksum } </td>
                                </tr>
                            }).collect::<Html>()
                    } </tbody>
                </table>
            </>
        }
    }

    fn render_menu(&self) -> Html {
        let clear_onclick = self.props.parent.callback(|_| {
            let up_msg = UpMessage::Experiment(Request::ClearDroneSoftware);
            crate::Msg::SendUpMessage(up_msg)
        });
        let upload_onchange = self.link.callback(move |value| {
            let mut result = Vec::new();
            if let ChangeData::Files(files) = value {
                let files = js_sys::try_iter(&files)
                    .unwrap()
                    .unwrap()
                    .map(|v| File::from(v.unwrap()));
                result.extend(files);
            }
            Msg::AddSoftware(result)
        });
        html! {
            <>
                <input id="drone_file_upload" class="is-hidden" type="file" multiple=true onchange=upload_onchange />
                <footer class="card-footer">
                    <a class="card-footer-item"><label for="drone_file_upload">{ "Upload" }</label></a>
                    <a class="card-footer-item" onclick=clear_onclick>{ "Clear" }</a>
                </footer>
            </>
        }
    }
}
