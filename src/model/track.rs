use super::*;

#[derive(Debug, Serialize, Deserialize, Diff, Clone, PartialEq)]
pub struct Obstacle {
    pub index: usize,
    pub radius: f32,
    pub position: Vec2<f32>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ShapePoint {
    pub y: f32,
    pub left: f32,
    pub right: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Track {
    pub obstacles: Vec<Obstacle>,
    pub shape: Vec<ShapePoint>,
}

impl Track {
    pub fn new_from_env() -> Self {
        let seed = match std::env::var("SEED") {
            Ok(seed) => seed.parse().unwrap(),
            Err(_) => global_rng().gen(),
        };
        Self::new(seed)
    }
    pub fn new(seed: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        const TRACK_LEN: f32 = 1000.0;
        const TRACK_WIDTH: f32 = 20.0;
        const SAFE_MIDDLE: f32 = 5.0;
        const OBSTACLES_DENSITY: f32 = 0.2;

        let shape = {
            let mut shape = Vec::new();
            let mut y = 0.0;
            let mut left = Vec::new();
            let mut right = Vec::new();
            let mut ys = Vec::new();
            let mut mid = 0.0;
            while y < TRACK_LEN {
                ys.push(y);
                left.push(mid - TRACK_WIDTH);
                right.push(mid + TRACK_WIDTH);
                const DELTA: f32 = 10.0;
                y += DELTA;
                mid += rng.gen_range(-1.0..=1.0) * DELTA * 1.0;
            }
            let n = ys.len();
            let left = CardinalSpline::new(
                ys.iter()
                    .copied()
                    .zip(left.into_iter())
                    .map(|(y, x)| vec2(x, y))
                    .collect(),
                0.5,
            );
            let right = CardinalSpline::new(
                ys.iter()
                    .copied()
                    .zip(right.into_iter())
                    .map(|(y, x)| vec2(x, y))
                    .collect(),
                0.5,
            );
            for (left, right) in left
                .intervals()
                .into_iter()
                .zip(right.intervals().into_iter())
            {
                const N: usize = 10;
                for i in 0..N {
                    let left = left.get(i as f32 / N as f32);
                    let right = right.get(i as f32 / N as f32);
                    assert_eq!(left.y, right.y);
                    shape.push(ShapePoint {
                        y: -left.y,
                        left: left.x,
                        right: right.x,
                    });
                }
            }
            shape
        };

        let list: Vec<String> = serde_json::from_reader(
            std::fs::File::open(static_path().join("obstacles.json")).unwrap(),
        )
        .unwrap();
        let obstacle_options: Vec<(usize, ObstacleConfig)> = list
            .into_iter()
            .map(|path| {
                serde_json::from_reader(
                    std::fs::File::open(static_path().join(format!("{}.json", path))).unwrap(),
                )
                .unwrap()
            })
            .enumerate()
            .collect();
        let mut obstacles: Vec<Obstacle> = Vec::new();
        'obstacles: for _ in 0..(TRACK_LEN * TRACK_WIDTH * OBSTACLES_DENSITY) as usize {
            let index = obstacle_options
                .choose_weighted(&mut rng, |(_, obstacle)| obstacle.spawn_weight)
                .unwrap()
                .0;
            let radius = obstacle_options[index].1.hitbox_radius / 20.0;
            let y = rng.gen_range(-TRACK_LEN..-Model::SPAWN_AREA);
            let (shape_left, shape_right) = Self::at_shape(&shape, y);
            let x = rng.gen_range(shape_left + radius..shape_right - radius);
            if (x - (shape_left + shape_right) / 2.0).abs() < SAFE_MIDDLE {
                continue 'obstacles;
            }
            let position = vec2(x, y);
            for obstacle in &obstacles {
                if (obstacle.position - position).len() < radius + obstacle.radius {
                    continue 'obstacles;
                }
            }
            obstacles.push(Obstacle {
                index,
                radius,
                position,
            });
        }
        obstacles.sort_by_key(|o| -r32(o.position.y));
        Self { shape, obstacles }
    }
    fn at_shape(shape: &Vec<ShapePoint>, y: f32) -> (f32, f32) {
        let idx = match shape.binary_search_by_key(&r32(-y), |point| r32(-point.y)) {
            Ok(idx) => idx,
            Err(idx) => idx - 1,
        }
        .min(shape.len() - 2);
        fn lerp(a: f32, b: f32, t: f32) -> f32 {
            a + (b - a) * t
        }
        let left = lerp(
            shape[idx].left,
            shape[idx + 1].left,
            (y - shape[idx].y) / (shape[idx + 1].y - shape[idx].y),
        );
        let right = lerp(
            shape[idx].right,
            shape[idx + 1].right,
            (y - shape[idx].y) / (shape[idx + 1].y - shape[idx].y),
        );
        (left, right)
    }
    pub fn at(&self, y: f32) -> (f32, f32) {
        Self::at_shape(&self.shape, y)
    }
}
