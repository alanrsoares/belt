//! `role=` locators against a real headless browser. Skips when none is
//! installed, since the locator logic runs as page JavaScript.

use webdriver::browser::{find_browser, BrowserInstance};

const PAGE: &str = "data:text/html,<dialog open>\
<label for='book'>Book name</label><input id='book' value='Dune'>\
<span id='mail'>Email address</span><input aria-labelledby='mail'>\
</dialog>";

#[test]
fn role_matches_inputs_by_label_for_and_aria_labelledby() {
    if find_browser(None).is_err() {
        eprintln!("skipped: no Chromium-family browser found");
        return;
    }
    let (_browser, mut cdp) = BrowserInstance::launch(None, true, None, &[]).unwrap();
    cdp.navigate(PAGE, 10_000).unwrap();

    // The first field is filled: its value must not hide its label.
    cdp.wait_for("role=textbox:Book name", 2_000).unwrap();
    cdp.wait_for("role=textbox:Email address", 2_000).unwrap();
    assert!(cdp.wait_for("role=textbox:Password", 500).is_err());
}
