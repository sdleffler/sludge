use {
    nalgebra::{storage::Storage, Vector, U3},
    serde::{Deserialize, Serialize},
    std::{
        mem,
        ops::{Add, AddAssign, Mul, MulAssign, Sub, SubAssign},
    },
};

pub use mint;

pub use nalgebra::{
    self as na, Affine2, Affine3, Complex, Isometry2, Isometry3, Matrix2, Matrix3, Matrix4,
    Orthographic3, Perspective3, Point2, Point3, Projective2, Projective3, Quaternion, RealField,
    Rotation2, Rotation3, Scalar, Similarity2, Similarity3, Transform2, Transform3, Translation2,
    Translation3, Unit, UnitComplex, UnitQuaternion, Vector2, Vector3, Vector4,
};

pub use ncollide2d::{
    self as nc2d,
    bounding_volume::{self, BoundingVolume, HasBoundingVolume, AABB},
    query::{self, DefaultTOIDispatcher, Proximity},
    shape::{Ball, Cuboid, ShapeHandle},
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Box2<T: Scalar> {
    pub mins: Point2<T>,
    pub extent: Vector2<T>,
}

impl<T: Scalar> Box2<T> {
    pub fn new(x: T, y: T, w: T, h: T) -> Self {
        Self {
            mins: Point2::new(x, y),
            extent: Vector2::new(w, h),
        }
    }

    pub fn center(self) -> Point2<T>
    where
        T: RealField,
    {
        self.mins + self.extent / na::convert::<_, T>(2.)
    }

    pub fn to_aabb(self) -> AABB<T>
    where
        T: RealField,
    {
        AABB::new(self.mins, self.mins + self.extent)
    }

    pub fn x(&self) -> T {
        self.mins.coords.x.clone()
    }

    pub fn y(&self) -> T {
        self.mins.coords.y.clone()
    }

    pub fn w(&self) -> T {
        self.extent.x.clone()
    }

    pub fn h(&self) -> T {
        self.extent.y.clone()
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

pub mod coords_2d {
    use super::*;

    pub fn to_grid_indices(grid_size: f32, aabb: &AABB<f32>) -> impl Iterator<Item = (i32, i32)> {
        let x_start = (aabb.mins.x / grid_size).floor() as i32;
        let x_end = (aabb.maxs.x / grid_size).ceil() as i32;
        let y_start = (aabb.mins.y / grid_size).floor() as i32;
        let y_end = (aabb.maxs.y / grid_size).ceil() as i32;

        (x_start..x_end).flat_map(move |i| (y_start..y_end).map(move |j| (i, j)))
    }
}

/// A velocity structure combining both the linear angular velocities of a point.
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct Velocity2<N: RealField> {
    /// The linear velocity.
    pub linear: Vector2<N>,
    /// The angular velocity.
    pub angular: N,
}

impl<N: RealField> Velocity2<N> {
    /// Create velocity from its linear and angular parts.
    #[inline]
    pub fn new(linear: Vector2<N>, angular: N) -> Self {
        Velocity2 { linear, angular }
    }

    /// Create a purely angular velocity.
    #[inline]
    pub fn angular(w: N) -> Self {
        Velocity2::new(na::zero(), w)
    }

    /// Create a purely linear velocity.
    #[inline]
    pub fn linear(vx: N, vy: N) -> Self {
        Velocity2::new(Vector2::new(vx, vy), N::zero())
    }

    /// Create a zero velocity.
    #[inline]
    pub fn zero() -> Self {
        Self::new(na::zero(), N::zero())
    }

    /// Computes the velocity required to move from `start` to `end` in the given `time`.
    pub fn between_positions(start: &Isometry2<N>, end: &Isometry2<N>, time: N) -> Self {
        let delta = end / start;
        let linear = delta.translation.vector / time;
        let angular = delta.rotation.angle() / time;
        Self::new(linear, angular)
    }

    /// Compute the displacement due to this velocity integrated during the time `dt`.
    pub fn integrate(&self, dt: N) -> Isometry2<N> {
        (*self * dt).to_transform()
    }

    /// Compute the displacement due to this velocity integrated during a time equal to `1.0`.
    ///
    /// This is equivalent to `self.integrate(1.0)`.
    pub fn to_transform(&self) -> Isometry2<N> {
        Isometry2::new(self.linear, self.angular)
    }

    /// This velocity seen as a slice.
    ///
    /// The linear part is stored first.
    #[inline]
    pub fn as_slice(&self) -> &[N] {
        self.as_vector().as_slice()
    }

    /// This velocity seen as a mutable slice.
    ///
    /// The linear part is stored first.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [N] {
        self.as_vector_mut().as_mut_slice()
    }

    /// This velocity seen as a vector.
    ///
    /// The linear part is stored first.
    #[inline]
    pub fn as_vector(&self) -> &Vector3<N> {
        unsafe { mem::transmute(self) }
    }

    /// This velocity seen as a mutable vector.
    ///
    /// The linear part is stored first.
    #[inline]
    pub fn as_vector_mut(&mut self) -> &mut Vector3<N> {
        unsafe { mem::transmute(self) }
    }

    /// Create a velocity from a vector.
    ///
    /// The linear part of the velocity is expected to be first inside of the input vector.
    #[inline]
    pub fn from_vector<S: Storage<N, U3>>(data: &Vector<N, U3, S>) -> Self {
        Self::new(Vector2::new(data[0], data[1]), data[2])
    }

    /// Create a velocity from a slice.
    ///
    /// The linear part of the velocity is expected to be first inside of the input slice.
    #[inline]
    pub fn from_slice(data: &[N]) -> Self {
        Self::new(Vector2::new(data[0], data[1]), data[2])
    }

    /// Compute the velocity of a point that is located at the coordinates `shift` relative to the point having `self` as velocity.
    #[inline]
    pub fn shift(&self, shift: &Vector2<N>) -> Self {
        Self::new(
            self.linear + Vector2::new(-shift.y, shift.x) * self.angular,
            self.angular,
        )
    }

    /// Rotate each component of `self` by `rot`.
    #[inline]
    pub fn rotated(&self, rot: &Rotation2<N>) -> Self {
        Self::new(rot * self.linear, self.angular)
    }

    /// Transform each component of `self` by `iso`.
    #[inline]
    pub fn transformed(&self, iso: &Isometry2<N>) -> Self {
        Self::new(iso * self.linear, self.angular)
    }
}

impl<N: RealField> Add<Velocity2<N>> for Velocity2<N> {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self {
        Velocity2::new(self.linear + rhs.linear, self.angular + rhs.angular)
    }
}

impl<N: RealField> AddAssign<Velocity2<N>> for Velocity2<N> {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.linear += rhs.linear;
        self.angular += rhs.angular;
    }
}

impl<N: RealField> Sub<Velocity2<N>> for Velocity2<N> {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Velocity2::new(self.linear - rhs.linear, self.angular - rhs.angular)
    }
}

impl<N: RealField> SubAssign<Velocity2<N>> for Velocity2<N> {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.linear -= rhs.linear;
        self.angular -= rhs.angular;
    }
}

impl<N: RealField> Mul<N> for Velocity2<N> {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: N) -> Self {
        Velocity2::new(self.linear * rhs, self.angular * rhs)
    }
}

impl<N: RealField> MulAssign<N> for Velocity2<N> {
    #[inline]
    fn mul_assign(&mut self, rhs: N) {
        *self = Velocity2::new(self.linear * rhs, self.angular * rhs);
    }
}
