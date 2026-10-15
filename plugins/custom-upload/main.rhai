// Custom Upload — sends the picture to your own HTTP endpoint (multipart/form-data) and copies the link.
//
// 1. Put your server's address in upload_url() and its host name in plugin.toml (net = [...]).
// 2. If the server wants a token, write it in headers(). It lives in this plugin only: plugins cannot see the
//    tokens configured under Settings > Uploads, and those cannot see this one.
// 3. The server's answer is expected to be the link as plain text. For a JSON answer use
//    json_get(r.body, "data.link") instead of r.body (see PLUGINS.md).

register_action("upload", "Upload to my server (plugin)");

fn upload_url() { "https://upload.example.com/api/upload" }

fn headers() { #{ "Authorization": "Bearer CHANGE-ME" } }

fn action_upload(img) {
    let r = http_upload(upload_url(), "file", "snapforge.png", img, headers());
    if r.status >= 200 && r.status < 300 {
        clipboard_set(r.body);
        notify("Uploaded, the link is in the clipboard");
    } else {
        notify("Upload failed: HTTP " + r.status);
    }
    img
}
