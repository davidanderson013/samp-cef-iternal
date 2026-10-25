use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use winapi::shared::d3d9::IDirect3DDevice9;
use winapi::shared::windef::{HWND, RECT};
use winapi::um::wingdi::RGNDATA;
use winapi::um::winnt::HRESULT;

use crate::browser::manager::{ExternalClient, Manager};
use crate::static_cell::StaticCell;

use client_api::gta::entity::CEntity;
use client_api::gta::menu_manager::CMenuManager;
use client_api::gta::rw::{self, rpworld::*, rwplcore::*};
use client_api::samp::objects::Object;
use client_api::samp::version::{Version, version};

use retour::GenericDetour;

static RENDER: StaticCell<Render> = StaticCell::new();

const REFERENCE_FRAMES: u64 = 10;
const RESET_FLAG_PRE: u8 = 0;
const RESET_FLAG_POST: u8 = 1;

const DRAWING_EVENT: usize = 0x58FAE0;
const SHUTDOWN_RW_EVENT: usize = 0x53BB80;
const OBJECT_RENDER: usize = 0x59FD50;

type DrawingEventFn = extern "C" fn();
type ShutdownRwEventFn = extern "C" fn();
type EntityRenderFn = extern "thiscall" fn(obj: *mut CEntity);
type DlRwRenderFn = extern "stdcall" fn(obj: *mut RwObject);
type PresentFn = unsafe extern "system" fn(
    *mut IDirect3DDevice9,
    *const RECT,
    *const RECT,
    HWND,
    *const RGNDATA,
) -> HRESULT;

struct FrameCounter {
    start_at: Instant,
    frames: u64,
    last_fps: u64,
}

struct Render {
    manager: Arc<Mutex<Manager>>,
    centity_render: GenericDetour<EntityRenderFn>,
    dl_rw_render: Option<GenericDetour<DlRwRenderFn>>,
    atomic_hooks: HashMap<usize, AtomicHook>,
    drawing_event: GenericDetour<DrawingEventFn>,
    shutdown_event: GenericDetour<ShutdownRwEventFn>,
    present: Option<GenericDetour<PresentFn>>,
    counter: FrameCounter,
    last_atomic_probe: Instant,
}

impl Render {
    fn get<'a>() -> Option<&'a mut Render> {
        unsafe { RENDER.get_mut() }
    }

    fn calc_frames(&mut self) -> Option<u64> {
        let counter = &mut self.counter;

        counter.frames += 1;

        if counter.frames == REFERENCE_FRAMES {
            let elapsed = counter.start_at.elapsed().as_millis() as u64;

            let fps = (REFERENCE_FRAMES * 1000) / elapsed;

            counter.last_fps = fps;
            counter.frames = 0;
            counter.start_at = Instant::now();

            return Some(fps);
        }

        None
    }
}

pub fn initialize(manager: Arc<Mutex<Manager>>) {
    tracing::debug!("initializing rendering hooks");

    if client_api::gta::d3d9::set_proxy(None, Some(on_reset)) {
        tracing::debug!("Direct3D device reset hook installed");
    } else {
        client_api::gta::d9_proxy::set_proxy(
            on_device_created,
            on_device_render,
            on_reset,
            on_device_destroy,
        );
        tracing::debug!("Direct3D device creation hook installed");
    }

    let centity_render = unsafe {
        let render_func: extern "thiscall" fn(*mut CEntity) = std::mem::transmute(OBJECT_RENDER);
        let centity_render = GenericDetour::new(render_func, centity_render).unwrap();

        centity_render.enable().unwrap();
        centity_render
    };

    let drawing_event = unsafe {
        let func: DrawingEventFn = std::mem::transmute(DRAWING_EVENT);
        let hook = GenericDetour::new(func, drawing_event).unwrap();

        hook.enable().unwrap();
        hook
    };

    let shutdown_event = unsafe {
        let func: ShutdownRwEventFn = std::mem::transmute(SHUTDOWN_RW_EVENT);
        let hook = GenericDetour::new(func, shutdown_event).unwrap();

        hook.enable().unwrap();
        hook
    };

    tracing::debug!("rendering hooks initialized");

    let counter = FrameCounter {
        start_at: Instant::now(),
        frames: 0,
        last_fps: 0,
    };

    let render = Render {
        manager,
        centity_render,
        dl_rw_render: None,
        atomic_hooks: HashMap::new(),
        drawing_event,
        shutdown_event,
        present: None,
        counter,
        last_atomic_probe: Instant::now(),
    };

    unsafe {
        RENDER.set(render);
    }

    try_install_present_hook();
    try_install_dl_rw_render_hook();
}

pub fn uninitialize() {
    unsafe {
        if let Some(render) = Render::get()
            && let Some(present) = render.present.as_ref()
        {
            let _ = present.disable();
        }

        if let Some(render) = Render::get()
            && let Some(dl_rw_render) = render.dl_rw_render.as_ref()
        {
            let _ = dl_rw_render.disable();
        }

        RENDER.take();
    }
}

fn on_render() {
    crate::app::mainloop();
}

fn on_device_created() {
    tracing::debug!("Direct3D device created with reset hook");
}

fn on_device_render(_: &mut IDirect3DDevice9) {
    if !try_install_present_hook() {
        render();
    }
}

fn on_device_destroy(_: &mut IDirect3DDevice9) {}

fn samp_device_offset() -> Option<usize> {
    match version() {
        Version::V03DL => Some(0x2AC9D0),
        Version::V037 => Some(0x21A0A8),
        Version::V037R3 => Some(0x26E888),
        _ => None,
    }
}

fn try_install_present_hook() -> bool {
    let Some(render) = Render::get() else {
        return false;
    };

    if render.present.is_some() {
        return true;
    }

    let Some(device_offset) = samp_device_offset() else {
        return false;
    };
    if !client_api::samp::is_loaded() {
        return false;
    }

    let hook = unsafe {
        let device_ptr =
            client_api::samp::handle().add(device_offset) as *const *mut IDirect3DDevice9;
        let device = device_ptr.read();

        if device.is_null() || (*device).lpVtbl.is_null() {
            return false;
        }

        let present = (*(*device).lpVtbl).Present;

        match GenericDetour::new(present, native_present) {
            Ok(hook) => hook,
            Err(error) => {
                tracing::warn!(%error, "cannot create native Direct3D Present hook");
                return false;
            }
        }
    };

    render.present = Some(hook);

    let result = unsafe { render.present.as_ref().unwrap().enable() };
    if let Err(error) = result {
        render.present.take();
        tracing::warn!(%error, "cannot enable native Direct3D Present hook");
        return false;
    }

    tracing::debug!("native Direct3D Present hook installed");
    true
}

fn try_install_dl_rw_render_hook() -> bool {
    const DL_RW_RENDER_OFFSET: usize = 0xB5E70;

    let Some(render) = Render::get() else {
        return false;
    };

    if render.dl_rw_render.is_some() {
        return true;
    }
    if version() != Version::V03DL || !client_api::samp::is_loaded() {
        return false;
    }

    let hook = unsafe {
        let original: DlRwRenderFn =
            std::mem::transmute(client_api::samp::handle().add(DL_RW_RENDER_OFFSET));
        match GenericDetour::new(original, dl_rw_render) {
            Ok(hook) => hook,
            Err(error) => {
                tracing::warn!(%error, "cannot create 0.3.DL object rendering hook");
                return false;
            }
        }
    };

    render.dl_rw_render = Some(hook);
    let result = unsafe { render.dl_rw_render.as_ref().unwrap().enable() };
    if let Err(error) = result {
        render.dl_rw_render.take();
        tracing::warn!(%error, "cannot enable 0.3.DL object rendering hook");
        return false;
    }

    tracing::debug!("0.3.DL object rendering hook installed");
    true
}

unsafe extern "system" fn native_present(
    device: *mut IDirect3DDevice9, source_rect: *const RECT, destination_rect: *const RECT,
    destination_window: HWND, dirty_region: *const RGNDATA,
) -> HRESULT {
    render();

    let Some(render) = Render::get() else {
        return 0x80004005_u32 as HRESULT;
    };
    let Some(present) = render.present.as_ref() else {
        return 0x80004005_u32 as HRESULT;
    };

    unsafe {
        present.call(
            device,
            source_rect,
            destination_rect,
            destination_window,
            dirty_region,
        )
    }
}

fn on_reset(_: &mut IDirect3DDevice9, reset_flag: u8) {
    let Some(render) = Render::get() else {
        return;
    };

    let mut manager = render.manager.lock();

    match reset_flag {
        RESET_FLAG_PRE => {
            manager.on_lost_device();
            drop(manager);
            crate::external::call_dxreset();
        }
        RESET_FLAG_POST => {
            manager.on_reset_device();
            let rect = crate::utils::client_rect();
            manager.resize(rect[0], rect[1]);
        }
        _ => {}
    }
}

pub fn render() {
    if let Some(render) = Render::get() {
        let fps = render.calc_frames();

        {
            let mut manager = render.manager.lock();

            if let Some(&fps) = fps.as_ref() {
                manager.update_fps(fps);
            }

            manager.do_not_draw(CMenuManager::is_menu_active());
            manager.draw();
        }
    }
}

fn on_destroy() {
    tracing::debug!("RenderWare is shutting down");

    if let Some(render) = Render::get() {
        let mut manager = render.manager.lock();
        manager.remove_views();
    }
}

struct RenderState {
    client: *mut ExternalClient,
    before: bool,
}

extern "C" fn drawing_event() {
    try_install_dl_rw_render_hook();

    if let Some(render) = Render::get() {
        render.drawing_event.call();

        if render.last_atomic_probe.elapsed() >= Duration::from_millis(250) {
            ensure_r3_atomic_hooks(render);
            render.last_atomic_probe = Instant::now();
        }
    }

    on_render();
    try_install_present_hook();
}

extern "stdcall" fn dl_rw_render(rwobject: *mut RwObject) {
    let Some(render) = Render::get() else {
        return;
    };
    let Some(original) = render.dl_rw_render.as_ref() else {
        return;
    };

    let mut manager = render.manager.lock();
    for browser in manager.external_browsers() {
        let browser_ptr = browser as *mut _;
        for &object_id in &browser.object_ids {
            let matches = Object::get(object_id)
                .and_then(|object| object.entity())
                .is_some_and(|entity| entity._base._base.rw_entity as *mut RwObject == rwobject);
            if !matches {
                continue;
            }

            let mut before = RenderState {
                client: browser_ptr,
                before: true,
            };
            replace_texture(rwobject, &mut before as *mut _ as *mut c_void);
            original.call(rwobject);
            let mut after = RenderState {
                client: browser_ptr,
                before: false,
            };
            replace_texture(rwobject, &mut after as *mut _ as *mut c_void);
            return;
        }
    }

    original.call(rwobject);
}

#[derive(Clone, Copy)]
struct AtomicHook {
    object_id: i32,
    original: RpAtomicCallBackRender,
}

fn ensure_r3_atomic_hooks(render: &mut Render) {
    if version() != Version::V037R3 {
        return;
    }

    let mut pending = Vec::new();

    {
        let mut manager = render.manager.lock();

        for browser in manager.external_browsers() {
            for &object_id in &browser.object_ids {
                if let Some(object) = Object::get(object_id) {
                    for atomic in object.render_atomics() {
                        pending.push((browser.browser.id(), object_id, atomic));
                    }
                }
            }
        }
    }

    let mut installed = false;

    for (browser_id, object_id, atomic) in pending {
        unsafe {
            let current = (*atomic).renderCallBack;

            if current.map(|callback| callback as usize)
                == Some(atomic_render as *const () as usize)
            {
                continue;
            }

            render.atomic_hooks.insert(
                atomic as usize,
                AtomicHook {
                    object_id,
                    original: current,
                },
            );
            (*atomic).renderCallBack = Some(atomic_render);
            installed = true;

            tracing::trace!(
                browser = browser_id,
                object = object_id,
                atomic = ?atomic,
                "R3 atomic rendering hook installed"
            );
        }
    }

    if installed
        && render.centity_render.is_enabled()
        && let Err(error) = unsafe { render.centity_render.disable() }
    {
        tracing::warn!(%error, "cannot disable fallback entity rendering hook");
    }
}

extern "C" fn shutdown_event() {
    on_destroy();

    if let Some(render) = Render::get() {
        render.shutdown_event.call();
    }
}

extern "thiscall" fn centity_render(obj: *mut CEntity) {
    render_entity(obj);
}

fn render_entity(obj: *mut CEntity) {
    if let Some(render) = Render::get() {
        let mut manager = render.manager.lock();
        let _entity = unsafe { &mut *obj };

        let browsers = manager.external_browsers();

        for browser in browsers {
            let browser_ptr = browser as *mut _; // должно быть safe
            for &object_id in &browser.object_ids {
                if let Some(object) = Object::get(object_id)
                    && let Some(obj_entity) = object.entity()
                    && obj == obj_entity as *mut _ as *mut CEntity
                {
                    let rwobject = obj_entity._base._base.rw_entity as *mut RwObject;

                    if !rwobject.is_null() {
                        let mut before_state = RenderState {
                            client: browser_ptr,
                            before: true,
                        };

                        let before_ptr = &mut before_state as *mut _ as *mut c_void;

                        replace_texture(rwobject, before_ptr);

                        render.centity_render.call(obj);

                        let mut after_state = RenderState {
                            client: browser_ptr,
                            before: false,
                        };

                        let after_ptr = &mut after_state as *mut _ as *mut c_void;

                        replace_texture(rwobject, after_ptr);

                        return;
                    }
                }
            }
        }

        render.centity_render.call(obj);
    }
}

extern "C" fn atomic_render(atomic: *mut RpAtomic) -> *mut RpAtomic {
    let Some(render) = Render::get() else {
        return atomic;
    };
    let Some(hook) = render.atomic_hooks.get(&(atomic as usize)).copied() else {
        return atomic;
    };

    let mut manager = render.manager.lock();
    let browser = manager
        .external_browsers()
        .iter_mut()
        .find(|browser| browser.object_ids.contains(&hook.object_id));

    let Some(browser) = browser else {
        drop(manager);

        unsafe {
            if !atomic.is_null() {
                (*atomic).renderCallBack = hook.original;
            }
        }
        render.atomic_hooks.remove(&(atomic as usize));

        return unsafe {
            hook.original
                .map(|callback| callback(atomic))
                .unwrap_or(atomic)
        };
    };

    unsafe {
        if atomic.is_null() || (*atomic).geometry.is_null() {
            return hook
                .original
                .map(|callback| callback(atomic))
                .unwrap_or(atomic);
        }

        let materials = (*(*atomic).geometry).matList.as_mut_slice();
        before_entity_render(materials, browser);
        let result = hook
            .original
            .map(|callback| callback(atomic))
            .unwrap_or(atomic);
        after_entity_render(materials, browser);
        result
    }
}

fn replace_texture(rwobject: *mut RwObject, render_state: *mut c_void) {
    unsafe {
        if (*rwobject).obj_type == rpCLUMP {
            rw::rpclump_for_all_atomics(rwobject as *mut _, Some(atomic_callback), render_state);
        } else {
            atomic_callback(rwobject as *mut _, render_state);
        }
    }
}

extern "C" fn atomic_callback(atomic: *mut RpAtomic, data: *mut c_void) -> *mut RpAtomic {
    unsafe {
        if !atomic.is_null() && !(*atomic).geometry.is_null() {
            let render = &mut *(data as *mut RenderState);

            if render.before {
                before_entity_render(
                    (*(*atomic).geometry).matList.as_mut_slice(),
                    &mut *render.client,
                );
            } else {
                after_entity_render(
                    (*(*atomic).geometry).matList.as_mut_slice(),
                    &mut *render.client,
                );
            }
        }
    }

    atomic
}

unsafe fn before_entity_render(materials: &mut [*mut RpMaterial], client: &mut ExternalClient) {
    for material in materials {
        let material = *material;
        unsafe {
            if material.is_null() {
                continue;
            }

            let texture = (*material).texture;

            if texture.is_null() {
                continue;
            }

            if !(*texture).name().contains(&client.texture) {
                continue;
            }

            let mut view = client.browser.view.lock();

            if view.rwtexture().is_none() && !(*texture).raster.is_null() {
                let raster = &mut *(*texture).raster;
                let width = (raster.width * client.scale) as usize;
                let height = (raster.height * client.scale) as usize;

                tracing::trace!(
                    browser = client.browser.id(),
                    texture = %client.texture,
                    width = raster.width,
                    height = raster.height,
                    scale = client.scale,
                    "object browser texture matched"
                );

                view.make_active();

                drop(view);

                client.browser.resize(width, height);
                client.browser.restore_hide_status();

                view = client.browser.view.lock();
            }

            if let Some(replace) = view.rwtexture() {
                client.origin_surface_props = (*material).surface_props.clone();

                (*material).surface_props.ambient = 16.0;
                (*material).surface_props.diffuse = 0.0;
                (*material).surface_props.specular = 0.0;

                client.origin_texture = (*material).texture;
                client.prev_replacement = replace.as_ptr();
                (*material).texture = replace.as_ptr();

                break; // replaced. do not replace another
            }
        }
    }
}

unsafe fn after_entity_render(materials: &mut [*mut RpMaterial], client: &mut ExternalClient) {
    for material in materials {
        let material = *material;
        unsafe {
            if material.is_null() {
                continue;
            }

            let texture = (*material).texture;

            if texture.is_null() || texture != client.prev_replacement {
                continue;
            }

            (*material).texture = client.origin_texture;
            (*material).surface_props = client.origin_surface_props.clone();

            break; //
        }
    }
}
