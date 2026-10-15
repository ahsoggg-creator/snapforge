// Watermark — draws a small text mark in the bottom-right corner.
// Change the text here; there are no settings screens to hide behind.

register_action("mark", "Add a watermark (plugin)");

fn mark_text() { "SnapForge" }

fn apply_mark(img) {
    let size = img.width() / 48;
    if size < 14 { size = 14; }
    if size > 64 { size = 64; }

    let dim = measure_text(mark_text(), size);
    let x = img.width() - dim[0] - 14;
    let y = img.height() - dim[1] - 10;

    // a soft shadow keeps the mark readable on light and dark backgrounds
    img.draw_text(x + 1, y + 1, mark_text(), size, [0, 0, 0, 120]);
    img.draw_text(x, y, mark_text(), size, [255, 255, 255, 220]);
    img
}

// After-capture panel: tick "Add a watermark (plugin)".
fn action_mark(img) { apply_mark(img) }

// Runs on every picture right before it is written to the screenshots folder.
fn on_before_save(img, info) { apply_mark(img) }
