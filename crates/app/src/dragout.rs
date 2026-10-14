//! Dragging a picture out of the editor window: an OLE drag of a real file, so it can be dropped into Explorer,
//! a chat window or any other application that accepts files.

use std::path::Path;

use windows::Win32::Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_COPY, DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{ILFree, SHCreateDataObject, SHParseDisplayName};
use windows::core::{BOOL, HRESULT, PCWSTR, implement};

#[implement(IDropSource)]
struct Source;

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape_pressed: BOOL, key_state: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape_pressed.as_bool() {
            DRAGDROP_S_CANCEL
        } else if key_state.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Starts dragging `path`. Blocks until the mouse button is released (drop or cancel).
pub fn drag_file(path: &Path) -> Result<(), String> {
    let wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: standard shell/OLE calls; the PIDL is freed after the data object (which copies it) is created.
    unsafe {
        let _ = OleInitialize(None);
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut pidl, 0, None).map_err(|e| e.message())?;
        let list = [pidl as *const ITEMIDLIST];
        let data: Result<IDataObject, _> = SHCreateDataObject(None, Some(&list), None);
        ILFree(Some(pidl));
        let data = data.map_err(|e| e.message())?;
        let source: IDropSource = Source.into();
        let mut effect = DROPEFFECT(0);
        let r = DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect);
        if r == DRAGDROP_S_DROP || r == DRAGDROP_S_CANCEL || r.is_ok() {
            Ok(())
        } else {
            Err(format!("drag failed: {r:?}"))
        }
    }
}
