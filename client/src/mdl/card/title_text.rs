use yew::prelude::*;
use yewtil::NeqAssign;

#[derive(Clone, Debug, PartialEq, Properties)]
pub struct Props {
    #[prop_or_default]
    pub children: Children,
    #[prop_or_default]
    pub class: Classes,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct TitleText {
    props: Props,
    node_ref: NodeRef,
}

impl Component for TitleText {
    type Message = ();
    type Properties = Props;

    fn create(props: Self::Properties, _link: ComponentLink<Self>) -> Self {
        TitleText { props, node_ref: NodeRef::default() }
    }

    fn update(&mut self, _msg: Self::Message) -> ShouldRender {
        false
    }

    fn change(&mut self, props: Self::Properties) -> ShouldRender {
        self.props.neq_assign(props)
    }

    fn view(&self) -> Html {
        html! {
            <h2 ref=self.node_ref.clone()
                    class=classes!("mdl-card__title-text", self.props.class.clone())>
                { self.props.text.clone() }
            </h2>
        }
    }

    fn rendered(&mut self, _first_render: bool) {
        if let Some(element) = self.node_ref.cast::<yew::web_sys::HtmlElement>() {
            crate::mdl::upgrade_element(element);
        }
    }
}