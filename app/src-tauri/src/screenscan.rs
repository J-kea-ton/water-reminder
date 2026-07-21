// 读取屏幕上其他 app 的窗口位置，给桌宠找一块没被挡的桌面空白处。
// 用 CGWindowListCopyWindowInfo，只取窗口边框矩形，不读窗口内容，
// 所以不需要「辅助功能」或「屏幕录制」授权。

#[cfg(target_os = "macos")]
pub fn other_app_window_rects(own_pid: i64) -> Vec<(f64, f64, f64, f64)> {
    use core_foundation::base::{CFGetTypeID, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::{CFNumber, CFNumberGetTypeID};
    use core_foundation::string::CFString;
    use core_graphics::window::{
        copy_window_info, kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly,
    };

    let mut out: Vec<(f64, f64, f64, f64)> = Vec::new();

    let info = match copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        0,
    ) {
        Some(v) => v,
        None => return out,
    };

    let k_layer = CFString::from_static_string("kCGWindowLayer");
    let k_pid = CFString::from_static_string("kCGWindowOwnerPID");
    let k_bounds = CFString::from_static_string("kCGWindowBounds");
    let k_alpha = CFString::from_static_string("kCGWindowAlpha");
    let (k_x, k_y) = (
        CFString::from_static_string("X"),
        CFString::from_static_string("Y"),
    );
    let (k_w, k_h) = (
        CFString::from_static_string("Width"),
        CFString::from_static_string("Height"),
    );

    for i in 0..info.len() {
        let item = match info.get(i) {
            Some(v) => v,
            None => continue,
        };
        let dict: CFDictionary<CFString, core_foundation::base::CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(*item as _) };

        let num = |d: &CFDictionary<CFString, core_foundation::base::CFType>,
                   key: &CFString|
         -> Option<f64> {
            let v = d.find(key)?;
            unsafe {
                if CFGetTypeID(v.as_CFTypeRef()) != CFNumberGetTypeID() {
                    return None;
                }
                CFNumber::wrap_under_get_rule(v.as_CFTypeRef() as _).to_f64()
            }
        };

        // 只要普通应用窗口（layer 0），跳过菜单栏、Dock、桌面图标等
        if num(&dict, &k_layer).unwrap_or(-1.0) as i64 != 0 {
            continue;
        }
        // 跳过我们自己的窗口（桌宠自己不算遮挡物）
        if num(&dict, &k_pid).unwrap_or(-1.0) as i64 == own_pid {
            continue;
        }
        // 跳过完全透明的窗口
        if num(&dict, &k_alpha).unwrap_or(1.0) < 0.05 {
            continue;
        }

        let bounds = match dict.find(&k_bounds) {
            Some(v) => v,
            None => continue,
        };
        let b: CFDictionary<CFString, core_foundation::base::CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(b_ref(&bounds)) };

        let (x, y, w, h) = match (
            num(&b, &k_x),
            num(&b, &k_y),
            num(&b, &k_w),
            num(&b, &k_h),
        ) {
            (Some(a), Some(b_), Some(c), Some(d)) => (a, b_, c, d),
            _ => continue,
        };
        // 忽略过小的窗口（阴影、提示等）
        if w < 80.0 || h < 60.0 {
            continue;
        }
        out.push((x, y, w, h));
    }
    out
}

#[cfg(target_os = "macos")]
fn b_ref(v: &core_foundation::base::CFType) -> core_foundation::dictionary::CFDictionaryRef {
    use core_foundation::base::TCFType;
    v.as_CFTypeRef() as core_foundation::dictionary::CFDictionaryRef
}

#[cfg(not(target_os = "macos"))]
pub fn other_app_window_rects(_own_pid: i64) -> Vec<(f64, f64, f64, f64)> {
    Vec::new()
}

