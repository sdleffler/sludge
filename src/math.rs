use serde::{Deserialize, Serialize};

pub use nalgebra::{
    self as na, Complex, Isometry2, Isometry3, Matrix2, Matrix3, Matrix4, Point2, Point3,
    Quaternion, RealField, Rotation2, Rotation3, Scalar, Similarity2, Similarity3, Transform2,
    Transform3, Translation2, Translation3, Unit, UnitComplex, UnitQuaternion, Vector2, Vector3,
};

pub use ncollide2d::{
    self as nc,
    bounding_volume::{self, AABB},
    shape::{Ball, Cuboid},
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Box2<T: Scalar> {
    pub origin: Point2<T>,
    pub extent: Vector2<T>,
}

impl<T: Scalar> Box2<T> {
    pub fn new(x: T, y: T, w: T, h: T) -> Self {
        Self {
            origin: Point2::new(x, y),
            extent: Vector2::new(w, h),
        }
    }

    pub fn center(self) -> Point2<T>
    where
        T: RealField,
    {
        self.origin + self.extent / na::convert::<_, T>(2.)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Box3<T: Scalar> {
    pub origin: Point3<T>,
    pub extent: Vector3<T>,
}

#[rustfmt::skip]
pub fn homogeneous_mat3_to_mat4(mat3: &Matrix3<f32>) -> Matrix4<f32> {
    Matrix4::new(
        mat3[(0, 0)], mat3[(0, 1)],           0., mat3[(0, 2)],
        mat3[(1, 0)], mat3[(1, 1)],           0., mat3[(1, 2)],
                    0.,           0.,           1.,           0.,
        mat3[(2, 0)], mat3[(2, 1)],           0., mat3[(2, 2)],
    )
}

pub fn smooth_subpixels(position: Point2<f32>, direction: Vector2<f32>) -> Point2<f32> {
    let mut pixel_pos = position;
    if direction.norm_squared() > 0. {
        if direction.x.abs() > direction.y.abs() {
            pixel_pos.x = position.x.round();
            pixel_pos.y =
                (position.y + (pixel_pos.x - position.x) * direction.y / direction.x).round();
        } else {
            pixel_pos.y = position.y.round();
            pixel_pos.x =
                (position.x + (pixel_pos.y - position.y) * direction.x / direction.y).round();
        }
    }

    pixel_pos
}
