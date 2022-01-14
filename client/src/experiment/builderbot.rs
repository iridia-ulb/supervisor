use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use yew::prelude::*;

use yew::services::reader::{File, FileData, ReaderService, ReaderTask};
use yew::{html, ChangeData, Component, ComponentLink, Html, ShouldRender};

use shared::experiment::{software::Software};

pub struct ConfigCard {
    link: ComponentLink<Self>,
    props: Props,
    tasks: HashMap<String, ReaderTask>,
}

#[derive(Clone, Properties)]
pub struct Props {
    pub software: Rc<RefCell<Software>>,
}

pub enum Msg {
    ClearSoftware,
    AddSoftware(String, Vec<u8>),
    ReadSoftware(Vec<File>),
}

// is it possible to just add a callback to the update method
impl Component for ConfigCard {
    type Message = Msg;
    type Properties = Props;

    fn create(props: Props, link: ComponentLink<Self>) -> Self {
        // if props contains a closure, I could use that to communicate with the actual instance
        ConfigCard { 
            props,
            link,
            tasks: Default::default(),
        }
    }

    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        match msg {
            Msg::ReadSoftware(files) => {
                let link = self.link.clone();
                let tasks = files.into_iter()
                    .filter_map(move |file| {
                        let filename = file.name();
                        let callback = 
                            link.callback(|FileData {name, content}| Msg::AddSoftware(name, content));
                        match ReaderService::read_file(file, callback) {
                            Ok(task) => Some((filename, task)),
                            Err(_) => None,
                        }
                    });
                self.tasks.extend(tasks);
            },
            Msg::ClearSoftware =>
                self.props.software.borrow_mut().clear(),
            Msg::AddSoftware(name, content) =>
                self.props.software.borrow_mut().add(name, content),
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
                            <p class="level-item subtitle is-size-4">{ "BuilderBot Configuration" }</p>
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
                        match &self.props.software.borrow().check_config() {
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
                        self.props.software.borrow().checksums().iter()
                            .map(|(name, checksum)| html! {
                                <tr>
                                    <td> { name } </td>
                                    <td> { format!("{:x}", checksum) } </td>
                                </tr>
                            }).collect::<Html>()
                    } </tbody>
                </table>
            </>
        }
    }

    fn render_menu(&self) -> Html {
        let clear_onclick = self.link.callback(|_| Msg::ClearSoftware);
        let add_onchange = self.link.callback(move |value| {
            let mut result = Vec::new();
            if let ChangeData::Files(files) = value {
                let files = js_sys::try_iter(&files)
                    .unwrap()
                    .unwrap()
                    .map(|v| File::from(v.unwrap()));
                result.extend(files);
            }
            Msg::ReadSoftware(result)
        });
        html! {
            <>
                <input id="builderbot_add_software" class="is-hidden" type="file" multiple=true onchange=add_onchange />
                <footer class="card-footer">
                    <label class="card-footer-item" for="builderbot_add_software">{ "Add" }</label>
                    <a class="card-footer-item" onclick=clear_onclick>{ "Clear" }</a>
                </footer>
            </>
        }
    }
}
