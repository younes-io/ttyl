fn main() {
    let page = devnull::model::LogPage::parse(include_str!("../fixtures/log-latest.json"))
        .expect("fixture parses");
    println!("{} commits in {:?}", page.entries.len(), page.log);
}
