use std::cell::RefCell;
use std::rc::Rc;
use yew::prelude::*;

use yew::{html, Component, ComponentLink, Html, ShouldRender};

use shared::experiment::{software::Software, Request};

use shared::BackEndRequest;

use crate::UserInterface;

pub mod builderbot;
pub mod drone;
pub mod pipuck;

pub struct Interface {
    link: ComponentLink<Self>,
    props: Props,
}

// what if properties was just drone::Instance itself?
#[derive(Clone, Properties)]
pub struct Props {
    pub parent: ComponentLink<UserInterface>,
    pub builderbot_software: Rc<RefCell<Software>>,
    pub drone_software: Rc<RefCell<Software>>,
    pub pipuck_software: Rc<RefCell<Software>>,
}

pub enum Msg {
    StartExperiment,
    StopExperiment,
}

impl Component for Interface {
    type Message = Msg;
    type Properties = Props;

    fn create(props: Props, link: ComponentLink<Self>) -> Self {
        props.parent.send_message(crate::Msg::SetControlConfigComp(link.clone()));
        Interface { 
            props,
            link,
        }
    }

    fn update(&mut self, message: Self::Message) -> ShouldRender {
        match message {
            Msg::StartExperiment => {
                let request = BackEndRequest::ExperimentRequest(Request::Start {
                    builderbot_software: self.props.builderbot_software.borrow().clone(),
                    pipuck_software: self.props.pipuck_software.borrow().clone(),
                    drone_software: self.props.drone_software.borrow().clone(),
                });
                self.props.parent.send_message(crate::Msg::SendRequest(request, None));
            },
            Msg::StopExperiment => {
                let request = BackEndRequest::ExperimentRequest(Request::Stop);
                self.props.parent.send_message(crate::Msg::SendRequest(request, None));
            },
        }
        false
    }

    fn change(&mut self, _: Self::Properties) -> ShouldRender {
        true
    }

    fn view(&self) -> Html {
        html! {
            <>
                <div class="column is-full-mobile is-full-tablet is-full-desktop is-half-widescreen is-one-third-fullhd">
                    <builderbot::ConfigCard software=self.props.builderbot_software.clone() />
                </div>
                <div class="column is-full-mobile is-full-tablet is-full-desktop is-half-widescreen is-one-third-fullhd">
                    <drone::ConfigCard software=self.props.drone_software.clone() />
                </div>
                <div class="column is-full-mobile is-full-tablet is-full-desktop is-half-widescreen is-one-third-fullhd">
                    <pipuck::ConfigCard software=self.props.pipuck_software.clone() />
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