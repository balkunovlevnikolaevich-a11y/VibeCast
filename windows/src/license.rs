//! Free build: Pro implementation and art are not part of this repository.
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW,MB_OK};
use windows::core::w;
pub fn active()->bool{false}
pub fn open_dialog(owner:HWND)->bool{
 unsafe{MessageBoxW(Some(owner),w!("Для активации персонажей установите VibeCast Pro. Код из бота вводится в настройках Pro-версии."),w!("VibeCast Free"),MB_OK);}false
}
