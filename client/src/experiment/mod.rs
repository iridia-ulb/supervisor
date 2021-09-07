use shared::UpMessage;
use web_sys::{Element, HtmlInputElement};
use yew::{prelude::*, services::ConsoleService, web_sys::HtmlTextAreaElement};

use yew::services::reader::{File, FileChunk, FileData, ReaderService, ReaderTask};
use yew::{html, ChangeData, Component, ComponentLink, Html, ShouldRender};

use shared::experiment::Request;

pub mod drone;
pub mod control;

