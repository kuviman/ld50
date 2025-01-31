use super::*;

#[derive(ugli::Vertex)]
pub struct Particle {
    i_pos: vec2<f32>,
    i_vel: vec2<f32>,
    i_time: f32,
    i_size: f32,
    i_opacity: f32,
}

fn can_detonate(x: f32) -> bool {
    x >= -1.0 && x < 2.0
}

pub struct Game {
    next_update: f64,
    framebuffer_size: vec2<usize>,
    touch_control: Option<vec2<f32>>,
    touches: HashSet<u64>,
    time: f32,
    volume: f32,
    explosion_time: Option<f32>,
    last_model_tick: u64,
    geng: Geng,
    assets: Rc<Assets>,
    player_id: Id,
    camera: geng::Camera2d,
    model: simple_net::Remote<Model>,
    players: Collection<Player>,
    interpolated_players: Collection<Player>,
    player_skin_renderers: HashMap<Id, skin::Renderer>,
    next_particle: f32,
    particles: ugli::VertexBuffer<Particle>,
    show_player_names: bool,
    explosion_particles: ugli::VertexBuffer<Particle>,
    quad_geometry: ugli::VertexBuffer<draw2d::Vertex>,
    ride_sound_effect: geng::SoundEffect,
    avalanche_sound_effect: geng::SoundEffect,
    music: Option<geng::SoundEffect>,
    spawn_particles: Vec<(f32, vec2<f32>)>,
    ui_camera: geng::Camera2d,
    ui_controller: ui::Controller,
    minimap_full: bool,
    transition: Option<geng::state::Transition>,
}

impl Drop for Game {
    fn drop(&mut self) {
        if let Some(effect) = &mut self.music {
            effect.stop();
        }
        self.ride_sound_effect.stop();
        self.avalanche_sound_effect.stop();
    }
}

#[derive(Debug, Copy, Clone)]
pub enum UiMessage {
    Menu,
}

impl Game {
    fn ui_buttons(&self) -> Vec<ui::Button<UiMessage>> {
        vec![ui::Button::new(
            "menu",
            (self
                .ui_camera
                .view_area(self.framebuffer_size.map(|x| x as f32))
                .transform
                * vec3(-1.0, 1.0, 1.0))
            .xy()
                + vec2(0.1, -0.5),
            0.4,
            0.0,
            UiMessage::Menu,
        )]
    }
    fn handle_ui(&mut self, message: UiMessage) {
        match message {
            UiMessage::Menu => {
                self.transition = Some(geng::state::Transition::Switch(Box::new(Lobby::new(
                    &self.geng,
                    &self.assets,
                    self.player_id,
                    self.model.clone(),
                ))));
            }
        }
    }
    pub fn new(
        geng: &Geng,
        assets: &Rc<Assets>,
        player_id: Id,
        name: Option<String>,
        config: Option<skin::Config>,
        model: simple_net::Remote<Model>,
    ) -> Self {
        Self {
            minimap_full: false,
            next_update: 0.0,
            music: Some(assets.music.play()),
            touches: HashSet::new(),
            touch_control: None,
            framebuffer_size: vec2(1, 1),
            interpolated_players: default(),
            time: 0.0,
            volume: preferences::load::<f32>("volume.json")
                .unwrap_or(0.5)
                .clamp(0.0, 1.0),
            show_player_names: true,
            explosion_time: None,
            geng: geng.clone(),
            assets: assets.clone(),
            camera: geng::Camera2d {
                center: vec2(0.0, 0.0),
                rotation: Angle::ZERO,
                fov: geng::Camera2dFov::Vertical(20.0),
            },
            spawn_particles: Vec::new(),
            player_id,
            last_model_tick: u64::MAX,
            player_skin_renderers: default(),
            players: {
                let mut result = Collection::new();
                if let (Some(name), Some(config)) = (name, config) {
                    result.insert(Player {
                        start_y: 0.0,
                        emote: None,
                        state: PlayerState::SpawnWalk,
                        id: player_id,
                        name,
                        config,
                        seen_no_avalanche: false,
                        ride_volume: 0.0,
                        position: vec2(
                            thread_rng().gen_range(
                                model.get().track.at(0.0).safe_left
                                    ..=model.get().track.at(0.0).safe_right,
                            ),
                            0.0,
                        ),
                        radius: 0.3,
                        rotation: Angle::ZERO,
                        input: vec2::ZERO,
                        velocity: vec2::ZERO,
                    });
                }
                result
            },
            model,
            ride_sound_effect: {
                let mut effect = assets.ride_sound.effect(geng.audio().default_type());
                effect.set_volume(0.0);
                effect.play();
                effect
            },
            avalanche_sound_effect: {
                let mut effect = assets.avalanche_sound.effect(geng.audio().default_type());
                effect.set_volume(0.0);
                effect.play();
                effect
            },
            particles: ugli::VertexBuffer::new_dynamic(geng.ugli(), vec![]),
            explosion_particles: ugli::VertexBuffer::new_dynamic(geng.ugli(), vec![]),
            quad_geometry: ugli::VertexBuffer::new_static(
                geng.ugli(),
                vec![
                    draw2d::Vertex {
                        a_pos: vec2(-1.0, -1.0),
                    },
                    draw2d::Vertex {
                        a_pos: vec2(1.0, -1.0),
                    },
                    draw2d::Vertex {
                        a_pos: vec2(1.0, 1.0),
                    },
                    draw2d::Vertex {
                        a_pos: vec2(-1.0, 1.0),
                    },
                ],
            ),
            next_particle: 0.0,
            ui_camera: geng::Camera2d {
                center: vec2(0.0, 0.0),
                rotation: Angle::ZERO,
                fov: geng::Camera2dFov::Vertical(10.0),
            },
            ui_controller: ui::Controller::new(geng, assets),
            transition: None,
        }
    }

    fn draw_texture(
        &self,
        framebuffer: &mut ugli::Framebuffer,
        texture: &ugli::Texture,
        transform: mat3<f32>,
        color: Rgba<f32>,
    ) {
        let framebuffer_size = framebuffer.size();
        ugli::draw(
            framebuffer,
            &self.assets.texture_program,
            ugli::DrawMode::TriangleFan,
            &self.quad_geometry,
            (
                ugli::uniforms! {
                    u_texture: texture,
                    u_model_matrix: transform,
                    u_color: color,
                },
                self.camera.uniforms(framebuffer_size.map(|x| x as f32)),
            ),
            &ugli::DrawParameters { ..default() },
        );
    }

    fn draw_shadow(
        &self,
        framebuffer: &mut ugli::Framebuffer,
        transform: mat3<f32>,
        color: Rgba<f32>,
    ) {
        let framebuffer_size = framebuffer.size();
        ugli::draw(
            framebuffer,
            &self.assets.shadow,
            ugli::DrawMode::TriangleFan,
            &ugli::VertexBuffer::new_dynamic(
                self.geng.ugli(),
                vec![
                    draw2d::Vertex {
                        a_pos: vec2(-1.0, -1.0),
                    },
                    draw2d::Vertex {
                        a_pos: vec2(1.0, -1.0),
                    },
                    draw2d::Vertex {
                        a_pos: vec2(1.0, 1.0),
                    },
                    draw2d::Vertex {
                        a_pos: vec2(-1.0, 1.0),
                    },
                ],
            ),
            (
                ugli::uniforms! {
                    u_model_matrix: transform,
                    u_color: color,
                },
                self.camera.uniforms(framebuffer_size.map(|x| x as f32)),
            ),
            &ugli::DrawParameters {
                blend_mode: Some(ugli::BlendMode::straight_alpha()),
                ..default()
            },
        );
    }

    fn draw_obstacle(
        &self,
        framebuffer: &mut ugli::Framebuffer,
        obstacle: &ObstacleAssets,
        transform: mat3<f32>,
        color: Rgba<f32>,
    ) {
        self.draw_texture(
            framebuffer,
            &obstacle.texture,
            transform
                * mat3::scale_uniform(1.0 / obstacle.config.hitbox_radius)
                * mat3::translate(-obstacle.config.hitbox_origin)
                * mat3::scale(obstacle.texture.size().map(|x| x as f32))
                * mat3::scale_uniform(0.5)
                * mat3::translate(vec2(1.0, 1.0)),
            color,
        );
    }

    fn draw_player(&self, framebuffer: &mut ugli::Framebuffer, player: &Player) {
        if let Some(renderer) = self.player_skin_renderers.get(&player.id) {
            renderer.draw(
                framebuffer,
                &self.camera,
                &self.model.get().config,
                &skin::DrawInstance {
                    position: player.position,
                    rotation: player.rotation,
                    velocity: player.velocity,
                    state: player.state,
                },
            );
        }
        if let Some((_, index)) = player.emote {
            self.draw_texture(
                framebuffer,
                &self.assets.emotes[index],
                mat3::translate(
                    player.position
                        + vec2(0.0, 1.8)
                        + vec2(
                            0.0,
                            match player.state {
                                PlayerState::Parachute { timer } => timer,
                                _ => 0.0,
                            } * 10.0
                                / self.model.get().config.player.parachute_time,
                        ),
                ) * mat3::scale_uniform(0.3),
                Rgba::WHITE,
            );
        }
    }
    fn play_sound(&self, sound: &geng::Sound, pos: vec2<f32>) {
        let mut effect = sound.effect(self.geng.audio().default_type());
        effect.set_volume(
            (1.0 - ((pos - self.camera.center).len() / 10.0).sqr()).max(0.0) as f32 * self.volume,
        );
        effect.play()
    }

    fn update_interpolated(&mut self, delta_time: f32) {
        self.interpolated_players
            .retain(|player| self.players.get(&player.id).is_some());
        for player in &self.players {
            if self.interpolated_players.get(&player.id).is_none() || player.id == self.player_id {
                self.interpolated_players.insert(player.clone());
            }
            let i = self.interpolated_players.get_mut(&player.id).unwrap();
            const EXPECTED_PING: f32 = 0.3;
            *i = Player {
                state: match (i.state, player.state) {
                    (
                        PlayerState::Parachute { timer: from },
                        PlayerState::Parachute { timer: to },
                    ) => Some(PlayerState::Parachute {
                        timer: from + (to - from) / EXPECTED_PING * delta_time,
                    }),
                    (_, PlayerState::Parachute { timer: to }) => {
                        Some(PlayerState::Parachute { timer: to })
                    }
                    (PlayerState::Parachute { timer: from }, _) => {
                        let value = from - delta_time;
                        if value > 0.0 {
                            Some(PlayerState::Parachute { timer: value })
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
                .unwrap_or(player.state),
                start_y: player.start_y,
                id: player.id,
                emote: player.emote,
                name: player.name.clone(),
                position: i.position + (player.position - i.position) / EXPECTED_PING * delta_time,
                config: player.config.clone(),
                radius: player.radius,
                rotation: i.rotation + (player.rotation - i.rotation) / EXPECTED_PING * delta_time,
                input: player.input,
                velocity: i.velocity + (player.velocity - i.velocity) / EXPECTED_PING * delta_time,
                seen_no_avalanche: player.seen_no_avalanche,
                ride_volume: player.ride_volume,
            };
        }
    }
    fn press_space(&mut self) {
        if let Some(my_player) = self.players.get_mut(&self.player_id) {
            if can_detonate(my_player.position.x) {
                self.model.send(Message::StartTheRace);
            }
            let model = self.model.get();
            if model.avalanche_position.is_some()
                && my_player.state == PlayerState::SpawnWalk
                && model.config.enable_parachute
            {
                let y = model.avalanche_position.unwrap()
                    - model.config.avalanche.start
                    - model.avalanche_speed * model.config.player.parachute_time;
                my_player.position = vec2(model.track.at(y).middle(), y);
                my_player.start_y = my_player.position.y;
                my_player.state = PlayerState::Parachute {
                    timer: model.config.player.parachute_time,
                };
            }
            if let PlayerState::Walk = my_player.state {
                my_player.state = PlayerState::Ride { timer: 100.0 };
            }
        }
    }
}

impl geng::State for Game {
    fn transition(&mut self) -> Option<geng::state::Transition> {
        let result = self.transition.take();
        if result.is_some() {
            self.model.send(Message::Disconnect);
        }
        result
    }
    fn handle_event(&mut self, event: geng::Event) {
        for message in self.ui_controller.handle_event(&event, self.ui_buttons()) {
            self.handle_ui(message);
        }
        if self.music.is_none() {
            self.music = Some(self.assets.music.play());
        }
        match event {
            geng::Event::KeyPress { key } => match key {
                geng::Key::M => {
                    self.minimap_full = !self.minimap_full;
                }
                geng::Key::Space => {
                    self.press_space();
                }
                geng::Key::H => {
                    self.show_player_names = !self.show_player_names;
                }
                geng::Key::R => {
                    self.players.get_mut(&self.player_id).unwrap().respawn();
                }
                geng::Key::Digit1 => {
                    self.players.get_mut(&self.player_id).unwrap().emote = Some((0.0, 0));
                }
                geng::Key::Digit2 => {
                    self.players.get_mut(&self.player_id).unwrap().emote = Some((0.0, 1));
                }
                geng::Key::Digit3 => {
                    self.players.get_mut(&self.player_id).unwrap().emote = Some((0.0, 2));
                }
                geng::Key::Digit4 => {
                    self.players.get_mut(&self.player_id).unwrap().emote = Some((0.0, 3));
                }
                _ => {}
            },
            geng::Event::MousePress {
                button: geng::MouseButton::Left,
            } => {
                if let Some(position) = self.geng.window().cursor_position() {
                    self.touch_control = Some(position.map(|x| x as f32));
                    if self.touch_control.unwrap().y > self.framebuffer_size.y as f32 / 2.0 {
                        self.press_space();
                    }
                }
            }
            geng::Event::TouchStart(touch) => {
                self.touches.insert(touch.id);
                self.touch_control = Some(touch.position.map(|x| x as f32));
                if self.touch_control.unwrap().y > self.framebuffer_size.y as f32 / 2.0 {
                    self.press_space();
                }
            }
            geng::Event::TouchMove(touch) => {
                self.touch_control = Some(touch.position.map(|x| x as f32));
            }
            geng::Event::TouchEnd(touch) => {
                self.touches.remove(&touch.id);
                if self.touches.is_empty() {
                    self.touch_control = None;
                }
            }
            geng::Event::CursorMove { position, .. } => {
                if self
                    .geng
                    .window()
                    .is_button_pressed(geng::MouseButton::Left)
                {
                    self.touch_control = Some(position.map(|x| x as f32));
                }
            }
            geng::Event::MouseRelease { .. } => {
                self.touch_control = None;
            }
            _ => {}
        }
    }
    fn update(&mut self, delta_time: f64) {
        self.next_update -= delta_time;
        while self.next_update < 0.0 {
            let delta_time = 1.0 / 200.0;
            self.next_update += delta_time;

            let delta_time = delta_time as f32;
            if self.geng.window().is_key_pressed(geng::Key::PageUp) {
                self.volume += delta_time;
                self.volume = self.volume.clamp(0.0, 1.0);
                preferences::save("volume.json", &self.volume);
            }
            if self.geng.window().is_key_pressed(geng::Key::PageDown) {
                self.volume -= delta_time;
                self.volume = self.volume.clamp(0.0, 1.0);
                preferences::save("volume.json", &self.volume);
            }

            self.time += delta_time;

            self.update_interpolated(delta_time);

            if let Some(time) = &mut self.explosion_time {
                *time += delta_time;
                if *time > 1.0 {
                    self.explosion_time = None;
                }
            }

            for (t, _) in &mut self.spawn_particles {
                *t += delta_time * 3.0;
            }
            self.spawn_particles.retain(|(t, _)| *t < 1.0);

            let mut sounds: Vec<(&[geng::Sound], vec2<f32>)> = Vec::new();

            if let Some(me) = self.players.get_mut(&self.player_id) {
                if let Some((time, _)) = &mut me.emote {
                    *time += delta_time;
                    if *time > 1.0 {
                        me.emote = None;
                    }
                }

                me.input = vec2::ZERO;

                if let Some(pos) = self.touch_control {
                    me.input.x += ((pos.x - self.framebuffer_size.x as f32 / 2.0)
                        / (self.framebuffer_size.x as f32 / 4.0))
                        .clamp(-1.0, 1.0);
                } else {
                    if self.geng.window().is_key_pressed(geng::Key::A)
                        || self.geng.window().is_key_pressed(geng::Key::ArrowLeft)
                    {
                        me.input.x -= 1.0;
                    }
                    if self.geng.window().is_key_pressed(geng::Key::D)
                        || self.geng.window().is_key_pressed(geng::Key::ArrowRight)
                    {
                        me.input.x += 1.0;
                    }
                    if self.geng.window().is_key_pressed(geng::Key::W)
                        || self.geng.window().is_key_pressed(geng::Key::ArrowUp)
                    {
                        me.input.y += 1.0;
                    }
                    if self.geng.window().is_key_pressed(geng::Key::S)
                        || self.geng.window().is_key_pressed(geng::Key::ArrowDown)
                    {
                        me.input.y -= 1.0;
                    }
                }
            }
            {
                let model = self.model.get();

                self.player_skin_renderers
                    .retain(|id, _| model.players.get(id).is_some());
                for player in &model.players {
                    let renderer =
                        self.player_skin_renderers
                            .entry(player.id)
                            .or_insert_with(|| {
                                skin::Renderer::new(&self.geng, &player.config, &self.assets)
                            });
                    renderer.update(delta_time);
                }

                // TODO: remove this copypasta
                let my_player = self.players.get(&self.player_id);
                let mut target_player = my_player;
                if my_player.is_none()
                    || (model.avalanche_position.is_some()
                        && my_player.unwrap().state == PlayerState::SpawnWalk)
                {
                    if let Some(player) = self
                        .interpolated_players
                        .iter()
                        .min_by_key(|player| r32(player.position.y))
                    {
                        target_player = Some(player);
                    }
                }
                if model.avalanche_position.is_some()
                    && target_player.is_some()
                    && target_player.unwrap().state == PlayerState::SpawnWalk
                {
                    target_player = None;
                }

                let target_center = if let Some(target_player) = target_player {
                    target_player.position + target_player.velocity * 0.5
                } else if let Some(position) = model.avalanche_position {
                    let position = (position - 5.0).min(0.0);
                    vec2(model.track.at(position).middle(), position)
                } else {
                    vec2(0.0, 0.0)
                };
                self.camera.center +=
                    (target_center - self.camera.center) * (3.0 * delta_time).min(1.0);

                if model.tick != self.last_model_tick {
                    self.last_model_tick = model.tick;
                    for player in &model.players {
                        if player.id != self.player_id {
                            if self.players.get(&player.id).is_none() {
                                self.spawn_particles.push((0.0, player.position));
                                let mut sfx = self
                                    .assets
                                    .spawn_sound
                                    .effect(self.geng.audio().default_type());
                                sfx.set_volume(self.volume);
                                sfx.play();
                            }
                            self.players.insert(player.clone());
                        }
                    }
                    for player in &self.players {
                        if player.id != self.player_id && model.players.get(&player.id).is_none() {
                            self.spawn_particles.push((0.0, player.position));
                            let mut sfx = self
                                .assets
                                .spawn_sound
                                .effect(self.geng.audio().default_type());
                            sfx.set_volume(self.volume);
                            sfx.play();
                        }
                    }
                    self.players.retain(|player| {
                        model.players.get(&player.id).is_some() || player.id == self.player_id
                    });
                }
                if model.avalanche_position.is_none() {
                    if let Some(player) = self.players.get_mut(&self.player_id) {
                        player.seen_no_avalanche = true;
                    }
                }
                if let Some(me) = self.players.get_mut(&self.player_id) {
                    if me.seen_no_avalanche
                        && model.avalanche_position.is_some()
                        && me.state == PlayerState::SpawnWalk
                    {
                        self.explosion_time = Some(0.0);
                        let mut sfx = self
                            .assets
                            .boom_sound
                            .effect(self.geng.audio().default_type());
                        sfx.set_volume(self.volume);
                        sfx.play();
                        me.state = PlayerState::Ride { timer: 100.0 };
                    }
                    if let PlayerState::Crash { timer, .. } = me.state {
                        if timer > 2.0 {
                            if model.avalanche_position.is_none()
                                || me.position.y > model.avalanche_position.unwrap()
                            {
                                self.model.send(Message::Score(
                                    ((me.start_y - me.position.y) * 100.0) as i32,
                                ));
                                me.respawn();
                            } else if model.config.auto_continue {
                                me.state = PlayerState::Ride { timer: 0.0 };
                            } else if model.config.enable_walk {
                                me.state = PlayerState::Walk;
                            }
                        }
                    }
                }
                for player in &mut self.players {
                    let shape_point = model.track.at(player.position.y);
                    match &mut player.state {
                        PlayerState::Parachute { timer } => {
                            *timer -= delta_time;
                            if *timer < 0.0 {
                                player.state = PlayerState::Ride { timer: 100.0 };
                            }
                        }
                        PlayerState::SpawnWalk => {
                            player.update_walk(&model.config.player, delta_time);
                            player.position.y = 0.0;
                            player.position.x = player.position.x.clamp(
                                -model.config.track.safe_middle * 2.0 + player.radius,
                                model.config.track.safe_middle * 2.0 - player.radius,
                            );
                        }
                        PlayerState::Walk => {
                            player.update_walk(&model.config.player, delta_time);
                            for obstacle in model
                                .track
                                .query_obstacles(player.position.y + 10.0, player.position.y - 10.0)
                            {
                                let delta_pos = player.position - obstacle.position;
                                let peneration = player.radius + obstacle.radius - delta_pos.len();
                                if peneration > 0.0 {
                                    let normal = delta_pos.normalize_or_zero();
                                    player.position += normal * peneration;
                                    player.velocity -= normal * vec2::dot(player.velocity, normal);
                                }
                            }
                            player.position.x = player.position.x.clamp(
                                shape_point.left + player.radius,
                                shape_point.right - player.radius,
                            );
                        }
                        PlayerState::Ride { .. } | PlayerState::Crash { .. } => {
                            if let PlayerState::Ride { timer } = &mut player.state {
                                *timer += delta_time;
                            }
                            player.update_riding(&model.config.player, delta_time);
                            for obstacle in model
                                .track
                                .query_obstacles(player.position.y + 10.0, player.position.y - 10.0)
                            {
                                let delta_pos = player.position - obstacle.position;
                                let peneration = player.radius + obstacle.radius - delta_pos.len();
                                if peneration > 0.0 {
                                    let normal = delta_pos.normalize_or_zero();
                                    player.position += normal * peneration;
                                    player.velocity -= normal * vec2::dot(player.velocity, normal);
                                    // TODO: copypasta
                                    if player.state.can_crash(&model.config) {
                                        player.state = PlayerState::Crash {
                                            timer: 0.0,
                                            ski_velocity: player.velocity,
                                            ski_rotation: player.rotation,
                                            crash_position: player.position,
                                        };
                                        sounds.push((&self.assets.crash_sounds, player.position));
                                    }
                                }
                            }
                            if player.position.x < shape_point.left + player.radius
                                || player.position.x > shape_point.right - player.radius
                            {
                                // TODO: copypasta
                                if player.state.can_crash(&model.config) {
                                    player.state = PlayerState::Crash {
                                        timer: 0.0,
                                        ski_velocity: player.velocity,
                                        ski_rotation: player.rotation,
                                        crash_position: player.position,
                                    };
                                    sounds.push((&self.assets.crash_sounds, player.position));
                                }
                            }
                            player.position.x = player.position.x.clamp(
                                shape_point.left + player.radius,
                                shape_point.right - player.radius,
                            );
                        }
                    }
                    player.position.y = player.position.y.min(0.0);
                    if let Some(position) = model.avalanche_position {
                        if player.position.y > position
                            && !matches!(
                                player.state,
                                PlayerState::SpawnWalk | PlayerState::Crash { .. }
                            )
                        {
                            player.state = PlayerState::Crash {
                                timer: 0.0,
                                ski_velocity: player.velocity,
                                ski_rotation: player.rotation,
                                crash_position: player.position,
                            };
                            sounds.push((&self.assets.crash_sounds, player.position));
                        }
                    }
                }
                self.next_particle -= delta_time;
                while self.next_particle < 0.0 {
                    self.next_particle += 1.0 / 100.0;
                    let mut particles = Vec::new();
                    for player in &self.interpolated_players {
                        if let PlayerState::Ride { .. } | PlayerState::Crash { .. } = player.state {
                            particles.push(Particle {
                                i_pos: player.position,
                                i_vel: vec2::ZERO,
                                i_time: self.time,
                                i_size: 0.2,
                                i_opacity: 1.0,
                            });
                            let normal = vec2(1.0, 0.0).rotate(player.rotation);
                            let force = vec2::dot(player.velocity, normal).abs();
                            particles.push(Particle {
                                i_pos: player.position,
                                i_vel: vec2(
                                    thread_rng().gen_range(-1.0..=1.0),
                                    thread_rng().gen_range(-1.0..=1.0),
                                ) / 2.0
                                    + player.velocity,
                                i_time: self.time,
                                i_size: 0.4,
                                i_opacity: 0.5 * force / model.config.player.max_speed,
                            });
                        }
                    }
                    self.particles.extend(particles);
                    if let Some(pos) = model.avalanche_position {
                        for _ in 0..10 {
                            self.particles.push(Particle {
                                i_pos: vec2(
                                    self.camera.center.x + thread_rng().gen_range(-20.0..=20.0),
                                    pos + thread_rng().gen_range(-3.0..=0.0),
                                ),
                                i_vel: vec2(
                                    thread_rng().gen_range(-1.0..=1.0),
                                    thread_rng().gen_range(-1.0..=1.0),
                                ),
                                i_time: self.time,
                                i_size: 0.4,
                                i_opacity: 0.5,
                            });
                        }
                    }
                }
            }
            self.particles
                .retain(|particle| particle.i_time > self.time - 1.0);
            for particle in &mut *self.particles {
                particle.i_pos += particle.i_vel * delta_time;
                particle.i_vel -= particle.i_vel.clamp_len(..=delta_time * 5.0);
            }
            self.explosion_particles
                .retain(|particle| particle.i_time > self.time - 1.0);
            for particle in &mut *self.explosion_particles {
                particle.i_pos += particle.i_vel * delta_time;
                particle.i_vel -= particle.i_vel.clamp_len(..=delta_time * 5.0);
            }
            if let Some(player) = self.players.get(&self.player_id) {
                self.model.send(Message::UpdatePlayer(player.clone()));
            }

            for _event in self.model.update() {
                // TODO handle
            }

            for (sounds, pos) in sounds {
                self.play_sound(sounds.choose(&mut thread_rng()).unwrap(), pos);
            }
        }
    }
    fn draw(&mut self, framebuffer: &mut ugli::Framebuffer) {
        self.framebuffer_size = framebuffer.size();
        let model = self.model.get();

        let my_player = self.players.get(&self.player_id);

        // TODO: remove this copypasta
        let mut target_player = my_player;
        if my_player.is_none()
            || (model.avalanche_position.is_some()
                && my_player.unwrap().state == PlayerState::SpawnWalk)
        {
            if let Some(player) = self
                .interpolated_players
                .iter()
                .min_by_key(|player| r32(player.position.y))
            {
                target_player = Some(player);
            }
        }
        if model.avalanche_position.is_some()
            && target_player.is_some()
            && target_player.unwrap().state == PlayerState::SpawnWalk
        {
            target_player = None;
        }

        let view_area = self.camera.view_area(framebuffer.size().map(|x| x as f32));

        let in_view = |position: vec2<f32>| -> bool {
            let position_in_view = view_area.transform.inverse() * position.extend(1.0);
            if position_in_view.x.abs() > 1.5 {
                return false;
            }
            if position_in_view.y.abs() > 1.5 {
                return false;
            }
            true
        };

        ugli::clear(framebuffer, Some(Rgba::WHITE), None, None);
        let view_width = view_area.bounding_box().width();
        self.geng.draw2d().draw2d(
            framebuffer,
            &self.camera,
            &draw2d::Quad::new(
                Aabb2::<f32>::point(vec2(self.camera.center.x, 0.0))
                    .extend_left(view_width)
                    .extend_right(view_width)
                    .extend_up(100.0),
                Rgba::opaque(145.0 / 255.0, 249.0 / 255.0, 1.0),
            ),
        );
        self.geng.draw2d().draw2d(
            framebuffer,
            &self.camera,
            &draw2d::TexturedQuad::new(
                Aabb2::<f32>::point(vec2::ZERO)
                    .extend_symmetric(self.assets.background.size().map(|x| x as f32) * 0.05),
                &self.assets.background,
            ),
        );
        {
            let texture = if model.avalanche_position.is_none() {
                &self.assets.detonator
            } else {
                &self.assets.detonator2
            };
            self.geng.draw2d().draw2d(
                framebuffer,
                &self.camera,
                &draw2d::TexturedQuad::new(
                    Aabb2::<f32>::point(vec2::ZERO)
                        .extend_positive(texture.size().map(|x| x as f32) * 0.05),
                    texture,
                ),
            );
        }
        if let Some(my_player) = &my_player {
            if model.avalanche_position.is_none() && can_detonate(my_player.position.x) {
                self.geng.draw2d().draw2d(
                    framebuffer,
                    &self.camera,
                    &draw2d::TexturedQuad::new(
                        Aabb2::<f32>::point(self.camera.center + vec2(0.0, 4.0)).extend_symmetric(
                            self.assets.detonate_text.size().map(|x| x as f32) * 0.05,
                        ),
                        &self.assets.detonate_text,
                    ),
                );
            }
        }

        let framebuffer_size = framebuffer.size();

        let c2 = Rgba::new(0.9, 0.9, 0.95, 0.0);
        let c1 = Rgba::new(0.9, 0.9, 0.95, 0.9);

        self.geng.draw2d().draw2d(
            framebuffer,
            &self.camera,
            &draw2d::Chain::new(
                Chain::new(
                    model
                        .track
                        .query_shape(
                            self.camera.center.y + self.camera.fov.value() * 2.0,
                            self.camera.center.y - self.camera.fov.value() * 2.0,
                        )
                        .iter()
                        .map(|point| vec2(point.safe_left, point.y))
                        .collect(),
                ),
                0.1,
                Rgba::new(0.0, 0.0, 0.0, 0.3),
                0,
            ),
        );
        self.geng.draw2d().draw2d(
            framebuffer,
            &self.camera,
            &draw2d::Chain::new(
                Chain::new(
                    model
                        .track
                        .query_shape(
                            self.camera.center.y + self.camera.fov.value() * 2.0,
                            self.camera.center.y - self.camera.fov.value() * 2.0,
                        )
                        .iter()
                        .map(|point| vec2(point.safe_right, point.y))
                        .collect(),
                ),
                0.1,
                Rgba::new(0.0, 0.0, 0.0, 0.3),
                0,
            ),
        );
        {
            const OFF: f32 = 2.0;
            let vs: Vec<_> = model
                .track
                .query_shape(
                    self.camera.center.y + self.camera.fov.value() * 2.0,
                    self.camera.center.y - self.camera.fov.value() * 2.0,
                )
                .windows(2)
                .flat_map(|window| {
                    let a = &window[0];
                    let b = &window[1];
                    let n = -(vec2(b.left, b.y) - vec2(a.left, a.y))
                        .rotate_90()
                        .normalize();
                    [
                        draw2d::TexturedVertex {
                            a_pos: vec2(a.left, a.y),
                            a_color: Rgba::WHITE,
                            a_vt: vec2(0.0, a.left_len / 2.0),
                        },
                        draw2d::TexturedVertex {
                            a_pos: vec2(a.left, a.y) + n * OFF,
                            a_color: Rgba::WHITE,
                            a_vt: vec2(1.0, a.left_len / 2.0),
                        },
                    ]
                })
                .collect();
            if vs.len() >= 3 {
                self.geng.draw2d().draw2d(
                    framebuffer,
                    &self.camera,
                    &draw2d::TexturedPolygon::strip(vs, &self.assets.border),
                );
            }
            let vs: Vec<_> = model
                .track
                .query_shape(
                    self.camera.center.y + self.camera.fov.value() * 2.0,
                    self.camera.center.y - self.camera.fov.value() * 2.0,
                )
                .windows(2)
                .flat_map(|window| {
                    let a = &window[0];
                    let b = &window[1];
                    let n = (vec2(b.right, b.y) - vec2(a.right, a.y))
                        .rotate_90()
                        .normalize();
                    [
                        draw2d::TexturedVertex {
                            a_pos: vec2(a.right, a.y),
                            a_color: Rgba::WHITE,
                            a_vt: vec2(0.0, a.right_len / 2.0),
                        },
                        draw2d::TexturedVertex {
                            a_pos: vec2(a.right, a.y) + n * OFF,
                            a_color: Rgba::WHITE,
                            a_vt: vec2(1.0, a.right_len / 2.0),
                        },
                    ]
                })
                .collect();
            if vs.len() >= 3 {
                self.geng.draw2d().draw2d(
                    framebuffer,
                    &self.camera,
                    &draw2d::TexturedPolygon::strip(vs, &self.assets.border),
                );
            }
        }

        ugli::draw(
            framebuffer,
            &self.assets.particle_program,
            ugli::DrawMode::TriangleFan,
            ugli::instanced(&self.quad_geometry, &self.particles),
            (
                ugli::uniforms! {
                    u_time: self.time,
                    u_texture: &self.assets.particle,
                    u_color: Rgba::new(0.8, 0.8, 0.85, 0.7),
                },
                self.camera.uniforms(framebuffer_size.map(|x| x as f32)),
            ),
            &ugli::DrawParameters {
                blend_mode: Some(ugli::BlendMode::straight_alpha()),
                ..default()
            },
        );

        ugli::draw(
            framebuffer,
            &self.assets.particle_program,
            ugli::DrawMode::TriangleFan,
            ugli::instanced(&self.quad_geometry, &self.explosion_particles),
            (
                ugli::uniforms! {
                    u_time: self.time,
                    u_texture: &self.assets.particle,
                    u_color: Rgba::opaque(1.0, 0.5, 0.0),
                },
                self.camera.uniforms(framebuffer_size.map(|x| x as f32)),
            ),
            &ugli::DrawParameters {
                blend_mode: Some(ugli::BlendMode::straight_alpha()),
                ..default()
            },
        );

        for player in &self.interpolated_players {
            self.draw_shadow(
                framebuffer,
                mat3::translate(player.position) * mat3::scale_uniform(player.radius),
                Rgba::new(0.5, 0.5, 0.5, 0.5),
            );
        }
        if true {
            for obstacle in model.track.query_obstacles(
                self.camera.center.y + self.camera.fov.value() * 2.0,
                self.camera.center.y - self.camera.fov.value() * 2.0,
            ) {
                if !in_view(obstacle.position) {
                    continue;
                }
                self.draw_shadow(
                    framebuffer,
                    mat3::translate(obstacle.position) * mat3::scale_uniform(obstacle.radius),
                    Rgba::new(0.5, 0.5, 0.5, 0.5),
                );
            }
        }

        for player in &self.interpolated_players {
            self.draw_player(framebuffer, player);
        }

        for &(t, pos) in &self.spawn_particles {
            self.draw_texture(
                framebuffer,
                &self.assets.spawn,
                mat3::translate(pos + vec2(0.0, 0.5)) * mat3::scale_uniform(t),
                Rgba::new(0.5, 0.5, 1.0, 1.0 - t),
            );
        }

        for obstacle in model.track.query_obstacles(
            self.camera.center.y + self.camera.fov.value() * 2.0,
            self.camera.center.y - self.camera.fov.value() * 2.0,
        ) {
            if !in_view(obstacle.position) {
                continue;
            }
            self.draw_obstacle(
                framebuffer,
                &self.assets.obstacles[obstacle.index],
                mat3::translate(obstacle.position) * mat3::scale_uniform(obstacle.radius),
                Rgba::WHITE,
            );
        }

        if let Some(position) = model.avalanche_position {
            self.geng.draw2d().draw2d(
                framebuffer,
                &self.camera,
                &draw2d::Polygon::new_gradient(vec![
                    draw2d::ColoredVertex {
                        a_pos: vec2(self.camera.center.x - view_width, position - 3.0),
                        a_color: c2,
                    },
                    draw2d::ColoredVertex {
                        a_pos: vec2(self.camera.center.x - view_width, position),
                        a_color: c1,
                    },
                    draw2d::ColoredVertex {
                        a_pos: vec2(self.camera.center.x + view_width, position),
                        a_color: c1,
                    },
                    draw2d::ColoredVertex {
                        a_pos: vec2(self.camera.center.x + view_width, position - 3.0),
                        a_color: c2,
                    },
                ]),
            );
            self.geng.draw2d().draw2d(
                framebuffer,
                &self.camera,
                &draw2d::Quad::new(
                    Aabb2::point(vec2(self.camera.center.x, position))
                        .extend_left(50.0)
                        .extend_right(50.0)
                        .extend_up(100.0),
                    c1,
                ),
            );
        }
        if let Some(my_player) = &my_player {
            if model.avalanche_position.is_some()
                && matches!(my_player.state, PlayerState::SpawnWalk)
            {
                self.geng.draw2d().draw2d(
                    framebuffer,
                    &self.camera,
                    &draw2d::TexturedQuad::new(
                        Aabb2::<f32>::point(self.camera.center + vec2(0.0, -4.0)).extend_symmetric(
                            self.assets.spectating_text.size().map(|x| x as f32) * 0.05,
                        ),
                        &self.assets.spectating_text,
                    ),
                );
            } else if my_player.state == PlayerState::Walk {
                self.geng.draw2d().draw2d(
                    framebuffer,
                    &self.camera,
                    &draw2d::TexturedQuad::new(
                        Aabb2::<f32>::point(self.camera.center + vec2(0.0, -4.0))
                            .extend_symmetric(self.assets.walk.size().map(|x| x as f32) * 0.05),
                        &self.assets.walk,
                    ),
                );
            }
        }
        if self.show_player_names {
            for player in &self.interpolated_players {
                self.assets.font.draw(
                    framebuffer,
                    &self.camera,
                    player.position
                        + vec2(0.0, 1.0)
                        + vec2(
                            0.0,
                            match player.state {
                                PlayerState::Parachute { timer } => timer,
                                _ => 0.0,
                            } * 10.0
                                / model.config.player.parachute_time,
                        ),
                    0.5,
                    &player.name,
                    0.5,
                    Rgba::WHITE,
                );
            }
        }

        {
            // Minimap
            let minimap_camera = geng::Camera2d {
                center: self.camera.center + vec2(0.0, 0.0),
                rotation: Angle::ZERO,
                fov: geng::Camera2dFov::Vertical(300.0),
            };
            let mut texture = ugli::Texture::new_uninitialized(self.geng.ugli(), vec2(128, 128));
            texture.set_filter(ugli::Filter::Nearest);
            {
                let mut framebuffer = ugli::Framebuffer::new_color(
                    self.geng.ugli(),
                    ugli::ColorAttachment::Texture(&mut texture),
                );
                let framebuffer = &mut framebuffer;
                ugli::clear(framebuffer, Some(Rgba::TRANSPARENT_WHITE), None, None);
                let vs: Vec<_> = model
                    .track
                    .query_shape(
                        minimap_camera.center.y + minimap_camera.fov.value() * 2.0,
                        minimap_camera.center.y - minimap_camera.fov.value() * 2.0,
                    )
                    .windows(2)
                    .flat_map(|window| {
                        let a = &window[0];
                        [vec2(a.left, a.y), vec2(a.right, a.y)]
                    })
                    .collect();
                if vs.len() >= 3 {
                    self.geng.draw2d().draw2d(
                        framebuffer,
                        &minimap_camera,
                        &draw2d::Polygon::strip(vs, Rgba::BLACK),
                    );
                }
                let player_size = 5.0;
                for player in &self.interpolated_players {
                    if player.id == self.player_id {
                        continue;
                    }
                    self.geng.draw2d().draw2d(
                        framebuffer,
                        &minimap_camera,
                        &draw2d::Ellipse::circle(player.position, player_size, Rgba::BLUE),
                    );
                }
                if let Some(player) = my_player {
                    self.geng.draw2d().draw2d(
                        framebuffer,
                        &minimap_camera,
                        &draw2d::Ellipse::circle(player.position, player_size, Rgba::GREEN),
                    );
                }
                if let Some(avalanche_position) = model.avalanche_position {
                    self.geng.draw2d().draw2d(
                        framebuffer,
                        &minimap_camera,
                        &draw2d::Quad::new(
                            Aabb2::point(vec2(minimap_camera.center.x, avalanche_position))
                                .extend_symmetric(vec2(minimap_camera.fov.value(), 5.0)),
                            Rgba::RED,
                        ),
                    );
                }
            }
            if self.minimap_full {
                let size = framebuffer_size.y as f32;
                let size = vec2(size, size);
                self.geng.draw2d().draw2d(
                    framebuffer,
                    &geng::PixelPerfectCamera,
                    &draw2d::TexturedQuad::colored(
                        Aabb2::point(framebuffer_size.map(|x| x as f32) / 2.0)
                            .extend_symmetric(size / 2.0),
                        &texture,
                        Rgba::new(1.0, 1.0, 1.0, 0.2),
                    ),
                );
            } else {
                let size = framebuffer_size.y as f32 / 3.0;
                let size = vec2(size, size);
                self.geng.draw2d().draw2d(
                    framebuffer,
                    &geng::PixelPerfectCamera,
                    &draw2d::TexturedQuad::colored(
                        Aabb2::point(framebuffer_size.map(|x| x as f32) - size)
                            .extend_positive(size),
                        &texture,
                        Rgba::new(1.0, 1.0, 1.0, 0.5),
                    ),
                );
            }
        }

        if let Some(pos) = model.avalanche_position {
            let pos = pos - self.camera.center.y - self.camera.fov.value() / 2.0;
            let alpha = (1.0 - (pos - 1.0) / 5.0).clamp(0.0, 1.0);
            self.geng.draw2d().draw2d(
                framebuffer,
                &self.camera,
                &draw2d::TexturedQuad::colored(
                    Aabb2::<f32>::point(self.camera.center + vec2(0.0, 8.0))
                        .extend_symmetric(self.assets.ava_warning.size().map(|x| x as f32) * 0.05),
                    &self.assets.ava_warning,
                    Rgba::new(1.0, 1.0, 1.0, alpha),
                ),
            );
        } else if let Some((name, score)) = &model.winner {
            if self.show_player_names {
                self.assets.font.draw(
                    framebuffer,
                    &self.camera,
                    self.camera.center + vec2(0.0, 8.0),
                    1.0,
                    &format!("winner is {}", name),
                    0.5,
                    Rgba::WHITE,
                );
            }
            self.assets.font.draw(
                framebuffer,
                &self.camera,
                self.camera.center + vec2(0.0, 7.0),
                1.0,
                &format!("winner scored {}", score),
                0.5,
                Rgba::WHITE,
            );
        }
        if let Some(target_player) = target_player {
            if let PlayerState::Ride { .. } | PlayerState::Crash { .. } | PlayerState::Walk =
                &target_player.state
            {
                self.ride_sound_effect.set_volume(
                    (target_player.velocity.len() / model.config.player.max_speed * 0.05
                        + target_player.ride_volume.min(1.0) * 0.1)
                        * self.volume,
                );
                self.assets.font.draw(
                    framebuffer,
                    &self.camera,
                    self.camera.center + vec2(0.0, -9.0),
                    1.0,
                    &format!("score {}", target_player.score()),
                    0.5,
                    Rgba::WHITE,
                );
            } else {
                self.ride_sound_effect.set_volume(0.0);
            }
        } else {
            self.ride_sound_effect.set_volume(0.0);
        }
        if let Some(pos) = model.avalanche_position {
            self.avalanche_sound_effect.set_volume(
                (1.0 - ((pos - self.camera.center.y).abs() * 2.0 / self.camera.fov.value())
                    .powf(1.0))
                .clamp(0.0, 1.0)
                    * self.volume,
            );
        } else {
            self.avalanche_sound_effect.set_volume(0.0);
        }

        if let Some(music) = &mut self.music {
            music.set_volume(self.volume);
        }

        if let Some(time) = self.explosion_time {
            self.geng.draw2d().draw2d(
                framebuffer,
                &self.camera,
                &draw2d::TexturedQuad::colored(
                    Aabb2::<f32>::point(vec2(0.0, 5.0)).extend_symmetric(
                        self.assets.boom.size().map(|x| x as f32) * 0.05 * (1.0 + time),
                    ),
                    &self.assets.boom,
                    Rgba::new(1.0, 1.0, 1.0, 1.0 - time.sqr()),
                ),
            );
        }
        self.ui_controller
            .draw(framebuffer, &self.ui_camera, self.ui_buttons());
    }
}
