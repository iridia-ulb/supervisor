use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

use shared::UpMessage;
use yew::prelude::*;

use yew::{html, Component, ComponentLink, Html, ShouldRender};

use shared::experiment::{software::Software, Request};

use shared::BackEndRequest;

use crate::UserInterface;



pub mod drone;

pub struct Interface {
    link: ComponentLink<Self>,
    props: Props,
    pub drone_software: Rc<RefCell<Software>>,
    pub pipuck_software: Rc<RefCell<Software>>,
}

// what if properties was just drone::Instance itself?
#[derive(Clone, Properties)]
pub struct Props {
    pub parent: ComponentLink<UserInterface>,
}

pub enum Msg {
    UploadExperiment {
        start: bool,
    },
    StartExperiment,
    StopExperiment,
}

// is it possible to just add a callback to the update method
impl Component for Interface {
    type Message = Msg;
    type Properties = Props;

    fn create(props: Props, link: ComponentLink<Self>) -> Self {
        // are these messages necessary now?
        props.parent.send_message(crate::Msg::SetControlConfigComp(link.clone()));
        // if props contains a closure, I could use that to communicate with the actual instance
        Interface { 
            props,
            link,
            drone_software: Default::default(),
            pipuck_software: Default::default(),
        }
    }

    fn update(&mut self, message: Self::Message) -> ShouldRender {
        match message {
            Msg::UploadExperiment { start } => {
                let request = BackEndRequest::ExperimentRequest(Request::Configure {
                    pipuck_software: self.pipuck_software.borrow().clone(),
                    drone_software: self.drone_software.borrow().clone(),
                });
                let callback = if start {
                    Some(self.link.batch_callback(|result| match result {
                        Ok(_) => Some(Msg::StartExperiment),
                        Err(_) => None,
                    }))
                }
                else {
                    None
                };
                self.props.parent.send_message(crate::Msg::SendRequest(request, callback));
            },
            Msg::StartExperiment => {
                let request = BackEndRequest::ExperimentRequest(Request::Start);
                self.props.parent.send_message(crate::Msg::SendRequest(request, None));
            },
            Msg::StopExperiment => {
                let request = BackEndRequest::ExperimentRequest(Request::Stop);
                self.props.parent.send_message(crate::Msg::SendRequest(request, None));
            },
        }
        false
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
            <>
                <div class="column is-full-mobile is-full-tablet is-full-desktop is-half-widescreen is-one-third-fullhd">
                    <drone::ConfigCard software=self.drone_software.clone() />
                </div>
                <div class="column is-full-mobile is-full-tablet is-half-desktop is-third-widescreen is-one-quarter-fullhd">
                    <div class="card">
                    <header class="card-header">
                        <nav class="card-header-title is-shadowless has-background-white-ter level is-mobile">
                            <div class="level-left">
                                <p class="level-item subtitle is-size-4">{ "Control Panel" }</p>
                            </div>
                        </nav>
                    </header>
                    // <div class="card-content">
                    //     // <div class="content">
                            
                    //     // </div>
                    // </div>
                    <footer class="card-footer">
                        <a class="card-footer-item" 
                           onclick=self.link.callback(|_| Msg::StartExperiment)>{ "Start experiment" }</a>
                        <a class="card-footer-item" 
                           onclick=self.link.callback(|_| Msg::StopExperiment)>{ "Stop experiment" }</a>
                    </footer>
                    </div>
                </div>
            </>
            
        }
    }
}