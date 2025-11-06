mod error;
mod gfx;
mod input;
#[cfg_attr(windows, path = "os/windows/mod.rs")]
mod os;

use crate::gfx::text::FontRef;
use anyhow::Result;

use device_query::{DeviceState, Keycode};

use cgmath::prelude::*;
use gfx::text::ShaperSettings;
use harfrust::{Feature, Variation};
use parking_lot::Mutex;
use typed_arena::Arena;
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

use std::{
    borrow::Cow,
    cell::{LazyCell, OnceCell, RefCell},
    f32::consts::PI,
    str::FromStr,
    sync::{Arc, LazyLock, OnceLock},
    time::{Duration, Instant},
};

use crate::error::GameError;
use crate::gfx::GfxState;
use crate::gfx::text::FontCache;
use crate::gfx::text::FontShaper;
use crate::input::KeyboardState;
use crate::os::font_util;

const SIXTEEN_MILLIS: Duration = Duration::from_millis(16);

pub struct GameState {
    start_time: Instant,
    now: Instant,
    delta_time: Duration,
    run_time: Duration,
    state_number: u64,
    frame_number: u64,
    keyboard_state: KeyboardState,
    font_cache: FontCache,
    text: Option<String>,
    test_multiline: Option<[[f32; 3]; 5]>,
    should_quit: bool,
}

impl GameState {
    pub fn update(&mut self, now: Instant) -> Result<()> {
        self.delta_time = now - self.now;
        self.run_time += self.delta_time;
        self.now = now;
        self.state_number += 1;
        self.keyboard_state.update(self.frame_number);
        self.should_quit = self
            .keyboard_state
            .get_key_state(Keycode::LControl)
            .is_down()
            && self.keyboard_state.get_key_state(Keycode::Q).is_down();

        let slice_end = std::cmp::min(
            "Arrrrrrrrrrrrriverderci!".len(),
            (self.frame_number / 2) as usize,
        );
        self.text = Some("Arrrrrrrrrrrrriverderci!"[0..slice_end].to_string());
        //self.test_multiline = Some(get_multiline(self.run_time, surface_size_x, surface_size_y));
        Ok(())
    }

    #[inline]
    pub fn should_update(&self, now: &Instant) -> bool {
        self.frame_number == self.state_number && *now - self.now >= SIXTEEN_MILLIS
    }
}

#[derive(Default)]
struct App {
    window: Option<Arc<Window>>,
    gfx_state: Option<GfxState>,
    game_state: Option<GameState>,
}

impl App {
    fn new() -> Self {
        let device_state = DeviceState::new();
        let keyboard_state = KeyboardState::new(device_state);
        let now = Instant::now();
        let mut font_cache = FontCache::new();
        font_cache
            .load_system_fonts()
            .expect("Unable to load system fonts");
        font_cache
            .load_font_file("./fonts/SourceSerifVariable-Roman.ttf")
            .expect("Unable to load source serif variable font file");
        Self {
            window: None,
            gfx_state: None,
            game_state: Some(GameState {
                start_time: now,
                now,
                delta_time: Duration::from_millis(0),
                run_time: Duration::from_millis(0),
                frame_number: 0,
                state_number: 1,
                keyboard_state,
                font_cache,
                text: Some("Arrrrrrrrrrrrriverderci!".to_string()),
                test_multiline: None,
                should_quit: false,
            }),
        }
    }

    fn should_render(&self) -> bool {
        if let Some(game_state) = &self.game_state
            && self.window.is_some()
        {
            game_state.state_number > game_state.frame_number
        } else {
            false
        }
        //self.state_number > self.frame_number
    }

    fn window(&self) -> Option<&Arc<Window>> {
        self.window.as_ref()
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let window_attributes = Window::default_attributes()
                .with_title("Game")
                .with_inner_size(LogicalSize::new(1440.0, 900.0));
            let window = Arc::new(event_loop.create_window(window_attributes).unwrap());
            self.window = Some(window.clone());
            self.gfx_state = Some(GfxState::new(window.clone()));
            /*self.game_state.as_mut().unwrap().test_multiline = Some(get_multiline(
                Duration::from_millis(0),
                window.inner_size().width as f32,
                window.inner_size().height as f32,
            ));*/
            event_loop.set_control_flow(ControlFlow::Poll);
        }
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: winit::event::StartCause) {
        if cause == winit::event::StartCause::Init {
            return;
        }
        let game_state = match &mut self.game_state {
            Some(state) => state,
            None => return,
        };
        let now = Instant::now();
        if game_state.should_update(&now) {
            game_state.update(now).unwrap();
        }
        if self.should_render() {
            self.window().unwrap().request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let gfx_state = match &mut self.gfx_state {
            Some(state) => state,
            None => return,
        };

        let game_state = match &mut self.game_state {
            Some(state) => state,
            None => return,
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(physical_size) => {
                gfx_state.resize(Some(physical_size));
            }
            WindowEvent::RedrawRequested => match gfx_state.render(&game_state) {
                Ok(_) => game_state.frame_number += 1,
                Err(e) => match e.downcast_ref::<GameError>() {
                    // Reconfigure the surface if lost
                    Some(GameError::WgpuError(wgpu::SurfaceError::Lost)) => gfx_state.resize(None),
                    // Out of graphics memory probably means we should quit.
                    Some(GameError::WgpuError(wgpu::SurfaceError::OutOfMemory)) => {
                        event_loop.exit()
                    }
                    _ => eprintln!("{:?}", e),
                },
            },
            WindowEvent::ScaleFactorChanged { .. } => {
                gfx_state.resize(None);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let game_state = match &mut self.game_state {
            Some(state) => state,
            None => return,
        };

        if game_state.should_quit {
            event_loop.exit();
        }
    }
}

//static FONT_DATA: StaticCell<Mutex<Arena<u8>>> = StaticCell::new();
//static FONT_DATA: LazyLock<Mutex<Arena<u8>>> = LazyLock::new(|| Mutex::new(Arena::new()));
//static FONT_DATA_REFS: LazyLock<Vec<Arc<&'static [u8]>>> = LazyLock::new(|| Vec::new());

// static FONT_DATA: parking_lot::RwLock<Vec<Box<&'static [u8]>>> =
// parking_lot::RwLock::new(Vec::new());
//static FONT_REFS: Mutex<elsa::FrozenVec<WithRef<'_>>> = Mutex::new(elsa::FrozenVec::new());

//static FONT_DATA: OnceLock<Vec<&'static [u8]>> = OnceLock::new();

struct Cache {
    ref_data_collection: Vec<RefDataCollection<'static>>,
}

impl Cache {
    fn new() -> Self {
        Self {
            ref_data_collection: Vec::new(),
        }
    }

    fn add_data(&mut self, data: Vec<u8>) {
        static FONT_DATA: LazyLock<Mutex<Arena<&'static [u8]>>> =
            LazyLock::new(|| Mutex::new(Arena::new()));

        let _ = FONT_DATA.lock();
        let raw = unsafe { &*FONT_DATA.data_ptr() };
        let data = raw.alloc(data.leak());
        let wr = WithRef::from_data(data).with_with_ref();
        self.ref_data_collection
            .push(RefDataCollection { with_reff: wr });
    }

    /*fn add(&mut self, index: usize) {
        let bytes: &'static [u8] = &FONT_DATA.get_or_init(load_font_data)[index];
        let reff = WithRef::from_data(&bytes);
        let with_reff = WithRef::from_data(&bytes).with_with_ref();
        let ref_data = RefDataCollection { with_reff };

        //self.ref_data_collection.push(ref_data);

        /*let ref_data: Yoke<RefDataCollection<'static>, Arc<[u8]>> =
        Yoke::attach_to_cart(data, |rd| {
            let with_ref = WithRef::from_data(rd);
            let with_with_ref = with_ref.with_with_ref();
            RefDataCollection {
                reff: with_ref,
                with_reff: with_with_ref,
            }
        });*/
    }*/
}
#[derive(Clone)]
struct WithRef<'a> {
    data_ref: &'a [u8],
}

impl<'a> WithRef<'a> {
    fn from_data(data: &'a [u8]) -> Self {
        Self { data_ref: data }
    }

    fn with_with_ref(&self) -> WithWithRef<'a> {
        WithWithRef::new(self)
    }
}

#[derive(Clone)]
struct WithWithRef<'a> {
    with_ref: WithRef<'a>,
}

impl<'a> WithWithRef<'a> {
    fn new(reff: &WithRef<'a>) -> Self {
        Self {
            with_ref: reff.clone(),
        }
    }
}

struct RefDataCollection<'a> {
    //reff: WithRef<'a>,
    with_reff: WithWithRef<'a>,
}

/*impl RefDataCollection {
    fn from_data(data: Arc<[u8]>) -> Self {
        let reff: Yoke<WithRef<'static>, Arc<[u8]>> =
            Yoke::attach_to_cart(data.clone(), |rd| WithRef::from_data(rd));
        let with_reff: Yoke<WithWithRef<'static>, Arc<[u8]>> =
            Yoke::attach_to_cart(data.clone(), |rd| WithRef::from_data(rd).with_with_ref());
        Self { reff, with_reff }
    }
}*/

/*impl<'a> RefDataCollection<'a> {
    fn from_data(data: Arc<[u8]>) -> Yoke<RefDataCollection<'static>, Arc<[u8]>> {
        Yoke::attach_to_cart(data, |ref_data: &[u8]| {
            let reff = WithRef::from_data(ref_data);
            let with_reff = reff.with_with_ref();
            Self {
                data_ref: ref_data.clone(),
                reff,
                with_reff,
            }
        })
    }
}*/

/*impl<'zf> ZeroFrom<'zf, InnerCache<'_>> for InnerCache<'zf> {
    fn zero_from(other: &'zf InnerCache<'_>) -> Self {
        Self { refs: other.refs }
    }
}*/

/*impl<'zf> ZeroFrom<'zf, &'static mut Arena<u8>> for InnerCache<'zf> {
    fn zero_from(other: &'zf &'static mut Arena<u8>) -> Self {
        Self {
            refs: *other.iter_mut().map(|d| WithRef { data_ref: d }).collect(),
        }
    }
}*/

#[allow(unreachable_code)]
fn main() -> Result<()> {
    rayon::ThreadPoolBuilder::new().build_global()?;
    let system_fonts = font_util::load_system_font_paths()?;
    let mut font_cache = FontCache::new();

    let timer = Instant::now();

    let mut data_refs: Vec<Arc<[u8]>> = Vec::new();

    //let data_cache = FONT_DATA.init(Mutex::new(Arena::new()));

    //let data_ref_one: &[u8] = &*data_cache.alloc(vec![1, 2, 3, 4].into());
    //let data_ref_two = data_cache.alloc(vec![5, 6, 7, 8].into());

    //let data_ref_one = FONT_DATA.lock().unwrap().push_get(vec![1, 2, 3, 4]);
    //let data_ref_two = FONT_DATA.lock().unwrap().push_get(vec![5, 6, 7, 8]);

    let mut cache = Cache::new();
    for i in 1..5 {
        cache.add_data((i * 10 - 5..i * 10 + 10).collect());
    }

    //cache.add(0);
    //cache.add(1);

    // let data_ref_two = FONT_DATA.lock().unwrap().last().unwrap().clone();
    // data_refs.push(data_ref_two);

    /*let data_ref_one = FONT_DATA
        .lock()
        .unwrap()
        .alloc(vec![1, 2, 3, 4].into())
        .clone();
    data_refs.push(data_ref_one);

    let data_ref_two = FONT_DATA
        .lock()
        .unwrap()
        .alloc(vec![1, 2, 3, 4].into())
        .clone();
    data_refs.push(data_ref_two);*/

    //data_refs.push(data_ref_one);

    //let data_ref_one = data_cache.alloc_extend(vec![1, 2, 3, 4]);

    //FONT_REFS.push(Arc::new(data_ref_one));
    //let data_ref_two = data_cache.alloc_extend(vec![5, 6, 7, 8]);

    /*let mut result = 0;
    for font_path in &system_fonts {
        let _ = font_cache.load_font_file(font_path);
        result += 1;
    }*/
    {
        let result = font_cache.load_system_fonts()?;
        eprintln!(
            "cached {} system fonts in {} ms",
            result,
            timer.elapsed().as_millis()
        );
        //result.sort();
    }

    eprintln!(
        "font cache data size: {} MiB",
        font_cache.raw_data_size() as f32 / 1048576.0f32
    );

    //font_cache.list_fonts(true);

    font_cache.load_font_file("./fonts/SourceSerifVariable-Roman.ttf")?;
    font_cache.load_font_file("./fonts/westwood-studio/Westwood Studio.ttf")?;

    // eprintln!("{:?}", cascadia);
    // let source_serif_ref = font_cache.to_font_ref(&source_serif[0]);
    /*eprintln!(
        "{}",
        &font_cache
            .search_fonts("cascadia code")
            .iter()
            .map(|fcr| fcr.pretty_print())
            .collect::<Vec<String>>()
            .join("\n")
    );*/
    //eprintln!("{:?}", font_cache.search_fonts("times new roman"));
    // eprintln!("{:?}", font_cache.search_fonts("cambria"));
    // eprintln!("{:?}", font_cache.search_fonts("Yu Gothic"));

    //let cascadia = &font_cache.search_fonts("cascadia code regular")[0];

    let font = &font_cache.search_fonts("source serif variable regular")[0];
    //let font = &font_cache.search_fonts("westwood studio")[0];
    //let font = &font_cache.search_fonts("cambria regular")[0];

    let shaper_settings = ShaperSettings::new().with_features([
        Feature::from_str("+kern").unwrap(),
        Feature::from_str("+liga").unwrap(),
        Feature::from_str("+dlig").unwrap(),
    ]);

    let shaper = font.shaper(shaper_settings);
    let text = "fifi=>fi";
    //let glyph_buffer = shaper.shape(text, None, Default::default());

    /*for i in 0..glyph_buffer.len() {
        let info = glyph_buffer.glyph_infos()[i];
        let pos = glyph_buffer.glyph_positions()[i];
    }*/

    /*let mut font_face: FontShaper = FontShaper::new(
            source_serif_ref,
            Some(&[Variation::from(("Weight", 400.0f32))]),
            Some([
                harfrust::Feature::from_str("kern").unwrap(),
                harfrust::Feature::from_str("liga").unwrap(),
            ]),
        );
    h
        let text = "fififi";
        let _ = font_face.shape(text, None);*/

    //eprintln!("{}", source_serif_ref.pretty_print());
    //eprintln!("{:?}", cascadia_refs);
    env_logger::init();

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::new();

    event_loop.run_app(&mut app).unwrap();

    Ok(())
}

fn get_multiline(run_time: Duration, surface_size_x: f32, surface_size_y: f32) -> [[f32; 3]; 5] {
    let runtime_seconds: f32 = run_time.as_millis() as f32 / 1000.0;

    let t = runtime_seconds * 2.0;

    let moving_x_one = 0.25 + 0.125 * t.sin();
    let moving_x_two = 0.5 + 0.125 * t.cos();

    let aspect_ratio = surface_size_x / surface_size_y;

    let rotation: cgmath::Basis2<f32> = Rotation2::from_angle(cgmath::Rad(2.0 * PI * (t / 4.0)));
    let mut rotated_vector = rotation.rotate_vector(cgmath::Vector2::unit_x());
    rotated_vector.x /= aspect_ratio;

    let first = [0.0, 0.0, 0.0];
    let second = [moving_x_one, 0.5, 0.0];
    //let second = [0.5, 0.5, 0.0];
    let third = [moving_x_one, 0.0, 0.0];
    //let third = [0.5, 0.0, 0.0];
    let fourth = [moving_x_two, 0.5, 0.0];
    //let fourth = [0.75, 0.5, 0.0];

    let fifth = [
        moving_x_two + (0.125 * rotated_vector.x),
        0.5 + (0.125 * rotated_vector.y),
        0.0,
    ];

    [first, second, third, fourth, fifth]
}
