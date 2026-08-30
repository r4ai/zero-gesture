//! Main-thread-only AppKit and Core Animation implementation.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSFont, NSFontManager, NSFontTraitMask, NSFontWeightBlack,
    NSFontWeightBold, NSFontWeightHeavy, NSFontWeightLight, NSFontWeightMedium,
    NSFontWeightRegular, NSFontWeightSemibold, NSFontWeightThin, NSFontWeightUltraLight, NSScreen,
    NSStatusWindowLevel, NSView, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::{CFRetained, CFString, CFType, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColor, CGMutablePath};
use objc2_foundation::NSString;
use objc2_quartz_core::{
    kCAAlignmentCenter, kCALineCapRound, kCALineJoinRound, CAShapeLayer, CATextLayer, CATransaction,
};

use super::{Command, Shared};
use crate::domain::Point;
use crate::overlay::OverlayConfig;

const MAX_TRAIL_POINTS: usize = 64;
const LABEL_WIDTH: f64 = 240.0;

thread_local! {
    static OVERLAY: RefCell<Option<NativeOverlay>> = const { RefCell::new(None) };
}

pub(super) fn drain(shared: Arc<Shared>) {
    let Some(mtm) = MainThreadMarker::new() else {
        shared.failed.store(true, Ordering::Release);
        shared.wake.cancel();
        return;
    };
    loop {
        while let Ok(command) = shared.receiver.try_recv() {
            let shutdown = matches!(&command, Command::Shutdown);
            OVERLAY.with(|slot| apply(&mut slot.borrow_mut(), command, mtm));
            if shutdown {
                let _ = shared.shutdown_ack.try_send(());
            }
        }
        shared.wake.cancel();
        if shared.receiver.is_empty() || !shared.wake.request() {
            break;
        }
    }
}

fn apply(state: &mut Option<NativeOverlay>, command: Command, mtm: MainThreadMarker) {
    match command {
        Command::Start(config) => state
            .get_or_insert_with(NativeOverlay::new)
            .start(config, mtm),
        Command::Point(point) => {
            if let Some(overlay) = state {
                overlay.point(point, mtm);
            }
        }
        Command::Label(label) => {
            if let Some(overlay) = state {
                overlay.label(label, mtm);
            }
        }
        Command::End => {
            if let Some(overlay) = state {
                overlay.end();
            }
        }
        Command::Shutdown => {
            state.take();
        }
    }
}

struct NativeOverlay {
    signature: Vec<ScreenFrame>,
    trail: Vec<TrailWindow>,
    label: Option<LabelWindow>,
    points: TrailModel,
    config: Option<OverlayConfig>,
    active: bool,
}

impl NativeOverlay {
    fn new() -> Self {
        Self {
            signature: Vec::new(),
            trail: Vec::new(),
            label: None,
            points: TrailModel::new(),
            config: None,
            active: false,
        }
    }

    fn start(&mut self, config: OverlayConfig, mtm: MainThreadMarker) {
        self.clear_windows();
        self.signature = screen_frames(mtm);
        assert!(
            !self.signature.is_empty(),
            "macOS overlay requires a screen"
        );
        self.trail = self
            .signature
            .iter()
            .copied()
            .map(|frame| TrailWindow::new(frame, &config, mtm))
            .collect();
        self.points.clear();
        self.config = Some(config);
        self.active = true;
        for window in &self.trail {
            window.window.orderFrontRegardless();
        }
    }

    fn point(&mut self, point: Point, mtm: MainThreadMarker) {
        if !self.active {
            return;
        }
        self.reconcile_screens(mtm);
        self.points.push(point);
        let main_top = main_top(mtm);
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        for trail in &self.trail {
            trail.update(&self.points.points, main_top);
        }
        CATransaction::commit();
    }

    fn label(&mut self, label: Option<String>, mtm: MainThreadMarker) {
        if !self.active {
            return;
        }
        self.reconcile_screens(mtm);
        let Some((text, point)) = label_plan(label, self.points.last()) else {
            if let Some(label) = self.label.take() {
                label.window.orderOut(None);
            }
            return;
        };
        let Some(config) = self.config.as_ref() else {
            return;
        };
        if self.label.is_none() {
            self.label = Some(LabelWindow::new(config, mtm));
        }
        if let Some(label) = &self.label {
            label.show(&text, point, main_top(mtm));
        }
    }

    fn end(&mut self) {
        let label = terminalize_visual(&mut self.active, &mut self.points, &mut self.label);
        if let Some(label) = label {
            label.window.orderOut(None);
        }
        self.clear_windows();
        self.config = None;
    }

    fn reconcile_screens(&mut self, mtm: MainThreadMarker) {
        let current = screen_frames(mtm);
        if !display_changed(&mut self.signature, current, &mut self.points) {
            return;
        }
        assert!(
            !self.signature.is_empty(),
            "macOS overlay requires a screen"
        );
        self.clear_windows();
        let Some(config) = self.config.as_ref() else {
            return;
        };
        self.trail = self
            .signature
            .iter()
            .copied()
            .map(|frame| TrailWindow::new(frame, config, mtm))
            .collect();
        for window in &self.trail {
            window.window.orderFrontRegardless();
        }
    }

    fn clear_windows(&mut self) {
        for trail in self.trail.drain(..) {
            trail.window.orderOut(None);
        }
        if let Some(label) = self.label.take() {
            label.window.orderOut(None);
        }
    }
}

impl Drop for NativeOverlay {
    fn drop(&mut self) {
        self.clear_windows();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScreenFrame {
    x: u64,
    y: u64,
    width: u64,
    height: u64,
}

impl ScreenFrame {
    fn from_rect(frame: CGRect) -> Self {
        Self {
            x: frame.origin.x.to_bits(),
            y: frame.origin.y.to_bits(),
            width: frame.size.width.to_bits(),
            height: frame.size.height.to_bits(),
        }
    }
    fn rect(self) -> CGRect {
        CGRect::new(
            CGPoint::new(f64::from_bits(self.x), f64::from_bits(self.y)),
            CGSize::new(f64::from_bits(self.width), f64::from_bits(self.height)),
        )
    }
}

fn screen_frames(mtm: MainThreadMarker) -> Vec<ScreenFrame> {
    let screens = NSScreen::screens(mtm);
    (0..screens.count())
        .map(|index| ScreenFrame::from_rect(screens.objectAtIndex(index).frame()))
        .collect()
}

fn main_top(mtm: MainThreadMarker) -> f64 {
    NSScreen::mainScreen(mtm)
        .map(|screen| screen.frame().max_y())
        .unwrap_or(0.0)
}

struct TrailWindow {
    window: Retained<NSWindow>,
    layer: Retained<CAShapeLayer>,
    frame: ScreenFrame,
}

impl TrailWindow {
    fn new(frame: ScreenFrame, config: &OverlayConfig, mtm: MainThreadMarker) -> Self {
        let rect = frame.rect();
        let window = transparent_window(rect, mtm);
        let view = NSView::initWithFrame(mtm.alloc(), CGRect::new(CGPoint::ZERO, rect.size));
        let layer = CAShapeLayer::new();
        layer.setFrame(CGRect::new(CGPoint::ZERO, rect.size));
        layer.setContentsScale(window.backingScaleFactor());
        layer.setFillColor(None);
        layer.setStrokeColor(Some(&color(config.color, 1.0)));
        layer.setLineWidth(config.pen_width.max(1) as f64);
        layer.setLineCap(kCALineCapRound);
        layer.setLineJoin(kCALineJoinRound);
        view.setWantsLayer(true);
        view.setLayer(Some(&layer));
        window.setContentView(Some(&view));
        Self {
            window,
            layer,
            frame,
        }
    }

    fn update(&self, points: &VecDeque<Point>, main_top: f64) {
        let path = CGMutablePath::new();
        for (index, point) in points.iter().enumerate() {
            let local = local_point(*point, self.frame.rect(), main_top);
            unsafe {
                if index == 0 {
                    CGMutablePath::move_to_point(Some(&path), ptr::null(), local.x, local.y);
                } else {
                    CGMutablePath::add_line_to_point(Some(&path), ptr::null(), local.x, local.y);
                }
            }
        }
        self.layer.setPath(Some(&path));
    }
}

struct LabelWindow {
    window: Retained<NSWindow>,
    layer: Retained<CATextLayer>,
    size: CGSize,
}

impl LabelWindow {
    fn new(config: &OverlayConfig, mtm: MainThreadMarker) -> Self {
        let spec = LabelSpec::from_config(config);
        let size = spec.surface_size();
        let window = transparent_window(CGRect::new(CGPoint::ZERO, size), mtm);
        let view = NSView::initWithFrame(mtm.alloc(), CGRect::new(CGPoint::ZERO, size));
        let layer = CATextLayer::new();
        layer.setFrame(CGRect::new(CGPoint::ZERO, size));
        unsafe {
            layer.setAlignmentMode(kCAAlignmentCenter);
        }
        let font = resolve_font(&spec, mtm);
        let font_name = font.fontName();
        let cf_font_name: &CFString = (&*font_name).as_ref();
        let cf_font_name: &CFType = cf_font_name.as_ref();
        unsafe {
            layer.setFont(Some(cf_font_name));
        }
        layer.setFontSize(spec.size);
        layer.setForegroundColor(Some(&color((255, 255, 255), 1.0)));
        layer.setBackgroundColor(Some(&color((0, 0, 0), 0.72)));
        layer.setCornerRadius(spec.padding);
        view.setWantsLayer(true);
        view.setLayer(Some(&layer));
        window.setContentView(Some(&view));
        Self {
            window,
            layer,
            size,
        }
    }

    fn show(&self, label: &str, point: Point, main_top: f64) {
        let text = NSString::from_str(label);
        unsafe {
            self.layer.setString(Some(&text));
        }
        let center = CGPoint::new(point.x as f64, main_top - point.y as f64);
        self.window.setFrameOrigin(CGPoint::new(
            center.x - self.size.width / 2.0,
            center.y + 12.0,
        ));
        self.window.orderFrontRegardless();
    }
}

fn transparent_window(frame: CGRect, mtm: MainThreadMarker) -> Retained<NSWindow> {
    let policy = window_policy();
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            frame,
            policy.style,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    window.setOpaque(policy.opaque);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setIgnoresMouseEvents(policy.ignores_mouse);
    window.setHasShadow(false);
    window.setLevel(NSStatusWindowLevel);
    window.setCollectionBehavior(policy.collection);
    window
}

fn local_point(point: Point, frame: CGRect, main_top: f64) -> CGPoint {
    CGPoint::new(
        point.x as f64 - frame.origin.x,
        main_top - point.y as f64 - frame.origin.y,
    )
}

#[derive(Debug)]
struct TrailModel {
    points: VecDeque<Point>,
}
impl TrailModel {
    fn new() -> Self {
        Self {
            points: VecDeque::with_capacity(MAX_TRAIL_POINTS),
        }
    }
    fn push(&mut self, point: Point) {
        if self.points.len() == MAX_TRAIL_POINTS {
            self.points.pop_front();
        }
        self.points.push_back(point);
    }
    fn last(&self) -> Option<Point> {
        self.points.back().copied()
    }
    fn clear(&mut self) {
        self.points.clear();
    }
}

#[derive(Debug, PartialEq)]
struct TrailSpec {
    color: (u8, u8, u8),
    width: f64,
}
fn trail_spec(config: &OverlayConfig) -> TrailSpec {
    TrailSpec {
        color: config.color,
        width: config.pen_width.max(1) as f64,
    }
}

fn label_plan(label: Option<String>, latest: Option<Point>) -> Option<(String, Point)> {
    label.map(|text| (text, latest.unwrap_or(Point::new(0, 0))))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LabelWeight {
    UltraLight,
    Thin,
    Light,
    Regular,
    Medium,
    Semibold,
    Bold,
    Heavy,
    Black,
}

impl LabelWeight {
    fn from_config(weight: i32) -> Self {
        match weight.clamp(0, 1000) {
            0..=149 => Self::UltraLight,
            150..=249 => Self::Thin,
            250..=349 => Self::Light,
            350..=449 => Self::Regular,
            450..=549 => Self::Medium,
            550..=649 => Self::Semibold,
            650..=749 => Self::Bold,
            750..=849 => Self::Heavy,
            _ => Self::Black,
        }
    }
    fn manager_weight(self) -> isize {
        match self {
            Self::UltraLight => 2,
            Self::Thin => 3,
            Self::Light => 5,
            Self::Regular => 6,
            Self::Medium => 8,
            Self::Semibold => 9,
            Self::Bold => 11,
            Self::Heavy => 13,
            Self::Black => 14,
        }
    }
}

#[derive(Debug, PartialEq)]
struct LabelSpec {
    family: String,
    size: f64,
    weight: LabelWeight,
    padding: f64,
}
impl LabelSpec {
    fn from_config(config: &OverlayConfig) -> Self {
        Self {
            family: config.label_font_family.clone(),
            size: config.label_font_size.max(1) as f64,
            weight: LabelWeight::from_config(config.label_font_weight),
            padding: config.label_padding.max(0) as f64,
        }
    }
    fn surface_size(&self) -> CGSize {
        CGSize::new(LABEL_WIDTH, self.size + self.padding * 2.0)
    }
}

fn resolve_font(spec: &LabelSpec, mtm: MainThreadMarker) -> Retained<NSFont> {
    let family = NSString::from_str(&spec.family);
    NSFontManager::sharedFontManager(mtm)
        .fontWithFamily_traits_weight_size(
            &family,
            NSFontTraitMask::empty(),
            spec.weight.manager_weight(),
            spec.size,
        )
        .unwrap_or_else(|| NSFont::systemFontOfSize_weight(spec.size, system_weight(spec.weight)))
}

fn system_weight(weight: LabelWeight) -> f64 {
    unsafe {
        match weight {
            LabelWeight::UltraLight => NSFontWeightUltraLight,
            LabelWeight::Thin => NSFontWeightThin,
            LabelWeight::Light => NSFontWeightLight,
            LabelWeight::Regular => NSFontWeightRegular,
            LabelWeight::Medium => NSFontWeightMedium,
            LabelWeight::Semibold => NSFontWeightSemibold,
            LabelWeight::Bold => NSFontWeightBold,
            LabelWeight::Heavy => NSFontWeightHeavy,
            LabelWeight::Black => NSFontWeightBlack,
        }
    }
}

fn display_changed(
    signature: &mut Vec<ScreenFrame>,
    current: Vec<ScreenFrame>,
    trail: &mut TrailModel,
) -> bool {
    if *signature == current {
        return false;
    }
    *signature = current;
    trail.clear();
    true
}

fn terminalize_visual<T>(
    active: &mut bool,
    trail: &mut TrailModel,
    label: &mut Option<T>,
) -> Option<T> {
    *active = false;
    trail.clear();
    label.take()
}

#[derive(Debug, PartialEq, Eq)]
struct WindowPolicy {
    style: NSWindowStyleMask,
    collection: NSWindowCollectionBehavior,
    ignores_mouse: bool,
    opaque: bool,
}
fn window_policy() -> WindowPolicy {
    WindowPolicy {
        style: NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
        collection: NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
        ignores_mouse: true,
        opaque: false,
    }
}

fn color((red, green, blue): (u8, u8, u8), alpha: f64) -> CFRetained<CGColor> {
    CGColor::new_generic_rgb(
        f64::from(red) / 255.0,
        f64::from(green) / 255.0,
        f64::from(blue) / 255.0,
        alpha,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> OverlayConfig {
        OverlayConfig {
            color: (10, 20, 30),
            pen_width: 3,
            label_font_family: "system".to_string(),
            label_font_size: 14,
            label_font_weight: 400,
            label_padding: 4,
        }
    }

    #[test]
    fn native_overlay_window_policy_is_click_through_and_nonactivating() {
        let policy = window_policy();
        assert!(policy.style.contains(NSWindowStyleMask::Borderless));
        assert!(policy.style.contains(NSWindowStyleMask::NonactivatingPanel));
        assert!(policy.ignores_mouse);
        assert!(!policy.opaque);
        assert!(policy
            .collection
            .contains(NSWindowCollectionBehavior::CanJoinAllSpaces));
        assert!(policy
            .collection
            .contains(NSWindowCollectionBehavior::FullScreenAuxiliary));
    }

    #[test]
    fn trail_is_bounded_ordered_and_uses_pinned_appearance() {
        let mut trail = TrailModel::new();
        for x in 0..=MAX_TRAIL_POINTS {
            trail.push(Point::new(x as i32, 0));
        }
        assert_eq!(trail.points.len(), MAX_TRAIL_POINTS);
        assert_eq!(trail.points.front(), Some(&Point::new(1, 0)));
        assert_eq!(trail.points.back(), Some(&Point::new(64, 0)));
        assert_eq!(
            trail_spec(&config()),
            TrailSpec {
                color: (10, 20, 30),
                width: 3.0
            }
        );
    }

    #[test]
    fn label_uses_pinned_text_at_latest_point_and_hides_for_none() {
        assert_eq!(
            label_plan(Some("Copy".to_string()), Some(Point::new(8, 9))),
            Some(("Copy".to_string(), Point::new(8, 9)))
        );
        assert_eq!(label_plan(None, Some(Point::new(8, 9))), None);
        let spec = LabelSpec::from_config(&config());
        assert_eq!(
            spec,
            LabelSpec {
                family: "system".to_string(),
                size: 14.0,
                weight: LabelWeight::Regular,
                padding: 4.0
            }
        );
        assert_eq!(spec.surface_size(), CGSize::new(240.0, 22.0));
        assert_eq!(LabelWeight::from_config(-1), LabelWeight::UltraLight);
        assert_eq!(LabelWeight::from_config(700), LabelWeight::Bold);
        assert_eq!(LabelWeight::from_config(1001), LabelWeight::Black);
    }

    #[test]
    fn display_change_is_detected_and_clears_retained_trail() {
        let first = ScreenFrame {
            x: 0_f64.to_bits(),
            y: 0_f64.to_bits(),
            width: 100_f64.to_bits(),
            height: 100_f64.to_bits(),
        };
        let second = ScreenFrame {
            x: 100_f64.to_bits(),
            ..first
        };
        let mut signature = vec![first];
        let mut trail = TrailModel::new();
        trail.push(Point::new(1, 2));
        assert!(display_changed(&mut signature, vec![second], &mut trail));
        assert!(trail.points.is_empty());
        assert_eq!(signature, vec![second]);
    }

    #[test]
    fn overlay_end_clears_trail_and_releases_owned_label() {
        let mut active = true;
        let mut trail = TrailModel::new();
        trail.push(Point::new(1, 2));
        let mut label = Some("Copy".to_string());
        let removed = terminalize_visual(&mut active, &mut trail, &mut label);
        assert!(!active);
        assert!(trail.points.is_empty());
        assert!(label.is_none());
        assert_eq!(removed.as_deref(), Some("Copy"));
    }
}
