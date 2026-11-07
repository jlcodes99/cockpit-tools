use windows::Win32::{
  Foundation::{LRESULT, WPARAM},
  UI::WindowsAndMessaging as win32wm,
};

use crate::platform_impl::platform::event_loop::ProcResult;

pub fn is_msg_ime_related(msg_kind: u32) -> bool {
  matches!(
    msg_kind,
    win32wm::WM_IME_COMPOSITION
      | win32wm::WM_IME_COMPOSITIONFULL
      | win32wm::WM_IME_STARTCOMPOSITION
      | win32wm::WM_IME_ENDCOMPOSITION
      | win32wm::WM_IME_CHAR
      | win32wm::WM_CHAR
      | win32wm::WM_SYSCHAR
  )
}

pub struct MinimalIme {
  // True if we're currently receiving messages belonging to a finished IME session.
  getting_ime_text: bool,

  utf16parts: Vec<u16>,
}
impl Default for MinimalIme {
  fn default() -> Self {
    MinimalIme {
      getting_ime_text: false,
      utf16parts: Vec::with_capacity(16),
    }
  }
}
impl MinimalIme {
  pub(crate) fn process_message(
    &mut self,
    msg_kind: u32,
    wparam: WPARAM,
    more_char_coming: bool,
    result: &mut ProcResult,
  ) -> Option<String> {
    match msg_kind {
      win32wm::WM_IME_ENDCOMPOSITION => {
        self.getting_ime_text = true;
      }
      win32wm::WM_CHAR | win32wm::WM_SYSCHAR => {
        *result = ProcResult::Value(LRESULT(0));
        if self.getting_ime_text {
          self.utf16parts.push(wparam.0 as u16);

          if !more_char_coming {
            let result = String::from_utf16(&self.utf16parts).ok();
            self.utf16parts.clear();
            self.getting_ime_text = false;
            return result;
          }
        } else {
          return String::from_utf16(&[wparam.0 as u16]).ok();
        }
      }
      _ => (),
    }

    None
  }
}
