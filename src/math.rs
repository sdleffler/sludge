use {
    nalgebra::{storage::Storage, SimdPartialOrd, Vector, U3},
    num_traits::{Bounded, NumAssign, NumAssignRef, NumCast},
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

pub use num_traits as num;

pub trait Numeric:
    NumAssign + NumAssignRef + NumCast + Scalar + Copy + PartialOrd + SimdPartialOrd + Bounded
{
}
impl<T> Numeric for T where
    T: NumAssign + NumAssignRef + NumCast + Scalar + Copy + PartialOrd + SimdPartialOrd + Bounded
{
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Box2<N: Numeric> {
    pub mins: Point2<N>,
    pub maxs: Point2<N>,
}

impl<N: Numeric + RealField> From<ncollide2d::bounding_volume::AABB<N>> for Box2<N> {
    fn from(aabb: ncollide2d::bounding_volume::AABB<N>) -> Self {
        Self {
            mins: aabb.mins,
            maxs: aabb.maxs,
        }
    }
}

impl<N: Numeric> Box2<N> {
    pub fn new(x: N, y: N, w: N, h: N) -> Self {
        Self {
            mins: Point2::new(x, y),
            maxs: Point2::new(x + w, y + h),
        }
    }

    pub fn from_corners(mins: Point2<N>, maxs: Point2<N>) -> Self {
        Self { mins, maxs }
    }

    pub fn from_extents(mins: Point2<N>, extents: Vector2<N>) -> Self {
        Self {
            mins,
            maxs: mins + extents,
        }
    }

    pub fn from_half_extents(center: Point2<N>, half_extents: Vector2<N>) -> Self {
        Self {
            mins: center - half_extents,
            maxs: center + half_extents,
        }
    }

    pub fn invalid() -> Self {
        Self {
            mins: Vector2::repeat(N::max_value()).into(),
            maxs: Vector2::repeat(N::min_value()).into(),
        }
    }

    #[inline]
    pub fn is_valid(&self) -> bool {
        na::partial_le(&self.mins, &self.maxs)
    }

    #[inline]
    pub fn center(&self) -> Point2<N> {
        self.mins + self.half_extents()
    }

    #[inline]
    pub fn to_aabb(&self) -> ncollide2d::bounding_volume::AABB<N>
    where
        N: RealField,
    {
        ncollide2d::bounding_volume::AABB::new(self.mins, self.maxs)
    }

    #[inline]
    pub fn extents(&self) -> Vector2<N> {
        self.maxs.coords - self.mins.coords
    }

    #[inline]
    pub fn half_extents(&self) -> Vector2<N> {
        self.extents() / num::cast::<_, N>(2).unwrap()
    }

    #[inline]
    pub fn merge(&mut self, other: &Self) {
        *self = self.merged(other);
    }

    #[inline]
    pub fn merged(&self, other: &Self) -> Self {
        let new_mins = self.mins.coords.inf(&other.mins.coords);
        let new_maxes = self.mins.coords.sup(&other.maxs.coords);
        Self {
            mins: Point2::from(new_mins),
            maxs: Point2::from(new_maxes),
        }
    }

    #[inline]
    pub fn intersects(&self, other: &Self) -> bool {
        na::partial_le(&self.mins, &other.maxs) && na::partial_ge(&self.maxs, &other.mins)
    }

    #[inline]
    pub fn contains(&self, other: &Self) -> bool {
        na::partial_le(&self.mins, &other.mins) && na::partial_ge(&self.maxs, &other.maxs)
    }

    #[inline]
    pub fn loosen(&mut self, margin: N) {
        assert!(margin >= na::zero());
        let margin = Vector2::repeat(margin);
        self.mins = self.mins - margin;
        self.maxs = self.maxs + margin;
    }

    #[inline]
    pub fn loosened(&self, margin: N) -> Self {
        assert!(margin >= na::zero());
        let margin = Vector2::repeat(margin);
        Self {
            mins: self.mins - margin,
            maxs: self.maxs + margin,
        }
    }

    #[inline]
    pub fn tighten(&mut self, margin: N) {
        assert!(margin >= na::zero());
        let margin = Vector2::repeat(margin);
        self.mins = self.mins + margin;
        self.maxs = self.maxs - margin;
        assert!(na::partial_le(&self.mins, &self.maxs));
    }

    #[inline]
    pub fn tightened(&self, margin: N) -> Self {
        assert!(margin >= na::zero());
        let margin = Vector2::repeat(margin);
        Self {
            mins: self.mins + margin,
            maxs: self.maxs - margin,
        }
    }

    #[inline]
    pub fn from_points<'a, I>(pts: I) -> Self
    where
        I: IntoIterator<Item = &'a Point2<N>>,
    {
        let mut iter = pts.into_iter();

        let p0 = iter.next().expect("iterator must be nonempty");
        let mut mins: Point2<N> = *p0;
        let mut maxs: Point2<N> = *p0;

        for pt in iter {
            mins = mins.inf(&pt);
            maxs = maxs.sup(&pt);
        }

        Self { mins, maxs }
    }

    #[inline]
    pub fn transformed_by(&self, tx: &Matrix4<N>) -> Self
    where
        N: RealField,
    {
        let tl = Point3::new(self.mins.x, self.mins.y, N::zero());
        let tr = Point3::new(self.maxs.x, self.mins.y, N::zero());
        let br = Point3::new(self.maxs.x, self.maxs.y, N::zero());
        let bl = Point3::new(self.mins.x, self.maxs.y, N::zero());

        Self::from_points(&[
            tx.transform_point(&tl).xy(),
            tx.transform_point(&tr).xy(),
            tx.transform_point(&br).xy(),
            tx.transform_point(&bl).xy(),
        ])
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Box3<N: Scalar> {
    pub origin: Point3<N>,
    pub extent: Vector3<N>,
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

    pub fn to_grid_indices(grid_size: f32, aabb: &Box2<f32>) -> impl Iterator<Item = (i32, i32)> {
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
