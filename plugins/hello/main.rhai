// Hello — the smallest useful SnapForge plugin.
//
// Top-level statements run once when the plugin loads: that is where you register menu items, after-capture actions
// and editor tools. Functions named on_capture / on_before_save / on_after_save are hooks.

register_menu("hello", "Say hello");

// Plugins > Say hello
fn menu_hello() {
    notify("Hello from a SnapForge plugin!");
    log("the menu item was clicked");
}

// Runs after every capture. Returning nothing leaves the picture as it is.
fn on_capture(img) {
    log("captured " + img.width() + "x" + img.height() + " pixels");
}
