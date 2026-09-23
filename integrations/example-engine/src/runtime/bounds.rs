//! Conservative host bounds, independent of shader/material names.
use super::RuntimeError;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub minimum: [f32; 3],
    pub maximum: [f32; 3],
}

fn invalid(message: &str) -> RuntimeError {
    RuntimeError::PassPlan(format!("geometry bounds: {message}"))
}

impl Bounds {
    pub fn new(minimum: [f32; 3], maximum: [f32; 3]) -> Result<Self, RuntimeError> {
        if minimum.iter().chain(&maximum).any(|v| !v.is_finite())
            || (0..3).any(|axis| minimum[axis] > maximum[axis])
        {
            return Err(invalid("expected finite ordered limits"));
        }
        Ok(Self { minimum, maximum })
    }

    pub fn from_points(
        points: impl IntoIterator<Item = [f32; 3]>,
    ) -> Result<Option<Self>, RuntimeError> {
        let mut result: Option<Self> = None;
        for point in points {
            Self::new(point, point)?;
            if let Some(bounds) = &mut result {
                for (axis, value) in point.into_iter().enumerate() {
                    bounds.minimum[axis] = bounds.minimum[axis].min(value);
                    bounds.maximum[axis] = bounds.maximum[axis].max(value);
                }
            } else {
                result = Some(Self::new(point, point)?);
            }
        }
        Ok(result)
    }

    pub fn expand(self, amount: f32) -> Result<Self, RuntimeError> {
        if !amount.is_finite() || amount < 0.0 {
            return Err(invalid("expansion must be finite and nonnegative"));
        }
        if amount == 0.0 {
            return Self::new(self.minimum, self.maximum);
        }
        Self::new(
            self.minimum.map(|v| (v - amount).next_down()),
            self.maximum.map(|v| (v + amount).next_up()),
        )
    }

    pub fn center(self) -> [f32; 3] {
        std::array::from_fn(|axis| {
            f64::from(self.minimum[axis]).midpoint(f64::from(self.maximum[axis])) as f32
        })
    }

    fn corners(self) -> [[f32; 3]; 8] {
        std::array::from_fn(|corner| {
            std::array::from_fn(|axis| {
                if corner & (1 << axis) == 0 {
                    self.minimum[axis]
                } else {
                    self.maximum[axis]
                }
            })
        })
    }

    pub fn transform(self, model: &[f32; 16]) -> Result<Self, RuntimeError> {
        matrix(model)?;
        if [model[3], model[7], model[11], model[15]] != [0.0, 0.0, 0.0, 1.0] {
            return Err(invalid("world bounds require an affine object transform"));
        }
        let mut result = Self::from_points(self.corners().map(|p| {
            let transformed = multiply(
                model,
                [f64::from(p[0]), f64::from(p[1]), f64::from(p[2]), 1.0],
            );
            [
                transformed[0] as f32,
                transformed[1] as f32,
                transformed[2] as f32,
            ]
        }))?
        .expect("eight corners");
        // Include floating-point rounding in shader matrix evaluation.
        let slack = result
            .minimum
            .iter()
            .chain(&result.maximum)
            .map(|v| v.abs())
            .fold(1.0, f32::max)
            * 0.000001;
        result = result.expand(slack)?;
        Ok(result)
    }

    pub fn view_depth(self, view: &[f32; 16]) -> Result<f32, RuntimeError> {
        matrix(view)?;
        let center = self.center();
        let depth = -multiply(
            view,
            [
                f64::from(center[0]),
                f64::from(center[1]),
                f64::from(center[2]),
                1.0,
            ],
        )[2] as f32;
        if !depth.is_finite() {
            return Err(invalid("view depth overflow"));
        }
        Ok(depth)
    }

    /// Reject only boxes wholly outside one clip plane. Intersecting boxes,
    /// including those surrounding the camera, remain visible.
    pub fn visible(self, view: &[f32; 16], projection: &[f32; 16]) -> Result<bool, RuntimeError> {
        matrix(view)?;
        matrix(projection)?;
        let clip = self.corners().map(|p| {
            multiply(
                projection,
                multiply(
                    view,
                    [f64::from(p[0]), f64::from(p[1]), f64::from(p[2]), 1.0],
                ),
            )
        });
        for plane in 0..6 {
            if clip.iter().all(|p| {
                let distance = match plane {
                    0 => p[3] + p[0],
                    1 => p[3] - p[0],
                    2 => p[3] + p[1],
                    3 => p[3] - p[1],
                    4 => p[2],
                    5 => p[3] - p[2],
                    _ => unreachable!(),
                };
                let slack = p.iter().map(|v| v.abs()).fold(1.0, f64::max) * 0.00001;
                distance < -slack
            }) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[cfg(feature = "runtime")]
pub(crate) fn uniform_bytes(bounds: Option<Bounds>) -> [u8; 32] {
    let mut values = [0.0_f32; 8];
    if let Some(bounds) = bounds {
        values[..3].copy_from_slice(&bounds.minimum);
        values[3] = 1.0;
        values[4..7].copy_from_slice(&bounds.maximum);
    }
    let mut bytes = [0; 32];
    for (value, slot) in values.into_iter().zip(bytes.as_chunks_mut::<4>().0) {
        slot.copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn matrix(value: &[f32; 16]) -> Result<(), RuntimeError> {
    if value.iter().any(|v| !v.is_finite()) {
        return Err(invalid("matrix must be finite"));
    }
    Ok(())
}

fn multiply(matrix: &[f32; 16], point: [f64; 4]) -> [f64; 4] {
    std::array::from_fn(|row| {
        (0..4)
            .map(|column| f64::from(matrix[column * 4 + row]) * point[column])
            .sum()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const IDENTITY: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];

    #[test]
    fn expanded_bounds_reenter_the_frustum_before_visibility() {
        let bounds = Bounds::new([1.2, -0.1, 0.2], [1.3, 0.1, 0.4]).unwrap();
        assert!(!bounds.visible(&IDENTITY, &IDENTITY).unwrap());
        assert!(
            bounds
                .expand(0.3)
                .unwrap()
                .visible(&IDENTITY, &IDENTITY)
                .unwrap()
        );
        // Outward-rounded limits may move the computed midpoint by one ULP.
        assert!(
            bounds
                .center()
                .iter()
                .zip(bounds.expand(0.3).unwrap().center())
                .all(|(a, b)| (*a - b).abs() <= f32::EPSILON)
        );
        for amount in [-0.1, f32::NAN, f32::INFINITY] {
            assert!(bounds.expand(amount).is_err());
        }
        assert!(
            Bounds::new([f32::MAX; 3], [f32::MAX; 3])
                .unwrap()
                .expand(f32::MAX)
                .is_err()
        );
        let mut view = IDENTITY;
        view[14] = -4.0;
        assert!((bounds.view_depth(&view).unwrap() - 3.7).abs() < 0.00001);
    }

    #[test]
    fn transforms_cover_negative_scale_shear_and_translation() {
        let bounds = Bounds::new([-1.0, -2.0, -3.0], [1.0, 2.0, 3.0]).unwrap();
        let mut model = IDENTITY;
        model[0] = -2.0;
        model[4] = 3.0;
        model[12] = 10.0;
        let transformed = bounds.transform(&model).unwrap();
        assert!(transformed.minimum[0] <= 2.0 && transformed.maximum[0] >= 18.0);
        assert!(transformed.minimum[1] <= -2.0 && transformed.maximum[1] >= 2.0);
        assert!(Bounds::from_points([]).unwrap().is_none());
        assert!(Bounds::new([1.0; 3], [0.0; 3]).is_err());
        assert!(Bounds::from_points([[f32::NAN, 0.0, 0.0]]).is_err());
    }
}
