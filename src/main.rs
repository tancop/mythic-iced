use iced::Element;
use iced::widget::text;

mod colors;
mod decode;
mod epic;

fn main() {
    env_logger::init();

    iced::run(update, view).unwrap();
}

#[derive(Default)]
struct State;

enum Message {}

fn update(_: &mut State, _: Message) {}

fn view(_: &State) -> Element<'_, Message> {
    text!("hi there").into()
}
