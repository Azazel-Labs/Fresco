//! Example-engine G-buffer storage and tiled lighting resolve.
use super::RuntimeError;
use crate::profile::mesh::MeshSceneInputs;

fn invalid(reason: impl Into<String>) -> RuntimeError {
    RuntimeError::Deferred(reason.into())
}

/// General column-major inverse; supports perspective and orthographic host cameras.
pub fn inverse_view_projection(inputs: &MeshSceneInputs) -> Result<[f32; 16], RuntimeError> {
    let mut augmented = [[0.0_f64; 8]; 4];
    for (row, values) in augmented.iter_mut().enumerate() {
        for (col, value) in values[..4].iter_mut().enumerate() {
            *value = (0..4)
                .map(|k| {
                    f64::from(inputs.projection[k * 4 + row]) * f64::from(inputs.view[col * 4 + k])
                })
                .sum();
        }
        values[row + 4] = 1.0;
    }
    for col in 0..4 {
        let pivot = (col..4)
            .max_by(|a, b| {
                augmented[*a][col]
                    .abs()
                    .total_cmp(&augmented[*b][col].abs())
            })
            .expect("nonempty pivot range");
        augmented.swap(col, pivot);
        let scale = augmented[col][col];
        if !scale.is_finite() || scale.abs() < 1e-12 {
            return Err(invalid("view-projection matrix is singular or non-finite"));
        }
        for value in &mut augmented[col] {
            *value /= scale;
        }
        let pivot_values = augmented[col];
        for (row, values) in augmented.iter_mut().enumerate() {
            if row != col {
                let factor = values[col];
                for (value, pivot_value) in values.iter_mut().zip(pivot_values) {
                    *value -= factor * pivot_value;
                }
            }
        }
    }
    let mut inverse = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            inverse[col * 4 + row] = augmented[row][col + 4] as f32;
        }
    }
    if inverse.iter().any(|v| !v.is_finite()) {
        return Err(invalid("inverse camera matrix exceeds f32 range"));
    }
    Ok(inverse)
}
