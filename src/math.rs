use {
    nalgebra::SimdPartialOrd,
    num_traits::{Bounded, NumAssign, NumAssignRef, NumCast},
    rlua::prelude::*,
    serde::{de::DeserializeOwned, Deserialize, Serialize},
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
#[serde(into = "Box2Proxy<N>", from = "Box2Proxy<N>")]
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

#[derive(Serialize, Deserialize)]
#[serde(rename = "Box2")]
struct Box2Proxy<N: Numeric> {
    x: N,
    y: N,
    w: N,
    h: N,
}

impl<N: Numeric> From<Box2<N>> for Box2Proxy<N> {
    fn from(b: Box2<N>) -> Self {
        Self {
            x: b.mins.x,
            y: b.mins.y,
            w: b.maxs.x - b.mins.x,
            h: b.maxs.y - b.mins.y,
        }
    }
}

impl<N: Numeric> From<Box2Proxy<N>> for Box2<N> {
    fn from(b: Box2Proxy<N>) -> Self {
        Self::from_extents(Point2::new(b.x, b.y), Vector2::new(b.w, b.h))
    }
}

impl<'lua, N> ToLua<'lua> for Box2<N>
where
    N: Numeric + Serialize + ToLua<'lua>,
{
    fn to_lua(self, lua: LuaContext<'lua>) -> LuaResult<LuaValue<'lua>> {
        rlua_serde::to_value(lua, self)
    }
}

impl<'lua, N> FromLua<'lua> for Box2<N>
where
    N: Numeric + DeserializeOwned + FromLua<'lua>,
{
    fn from_lua(lua_value: LuaValue<'lua>, _lua: LuaContext<'lua>) -> LuaResult<Self> {
        rlua_serde::from_value(lua_value)
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
