use {anyhow::Result, nalgebra as na, rlua::prelude::*};

#[derive(Debug, Clone, Copy)]
pub struct Transform(pub na::Transform2<f32>);

impl LuaUserData for Transform {
    fn add_methods<'lua, M: LuaUserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method_mut("apply", |_, this, (rhs,): (Transform,)| {
            this.0 *= rhs.0;
            Ok(())
        });

        methods.add_method("clone", |_, &this, ()| Ok(this));

        methods.add_method("get_matrix", |_, this, ()| {
            Ok(this.0.matrix().as_slice().to_owned())
        });

        methods.add_method("inverse", |_, this, ()| {
            Ok(this.0.try_inverse().map(Transform))
        });

        methods.add_method(
            "inverse_transform_point",
            |ctx, this, (x, y): (f32, f32)| {
                let maybe_proj: Option<na::Projective2<f32>> = na::try_convert_ref(&this.0);
                match maybe_proj {
                    Some(proj) => {
                        let p = proj.inverse_transform_point(&na::Point2::new(x, y));
                        Ok((Some(p.x), LuaValue::Number(p.y as LuaNumber)))
                    }
                    None => Ok((
                        None,
                        LuaValue::String(ctx.create_string("non-invertible matrix")?),
                    )),
                }
            },
        );

        methods.add_method("is_affine_transform", |_ctx, this, ()| {
            Ok(na::is_convertible::<_, na::Affine2<f32>>(&this.0))
        });

        methods.add_method_mut("reset", |_ctx, this, ()| {
            this.0 = na::Transform2::identity();
            Ok(())
        });

        methods.add_method_mut("rotate", |_ctx, this, theta: f32| {
            this.0 *= na::Rotation2::new(theta);
            Ok(())
        });

        methods.add_method_mut("scale", |_ctx, this, factor: f32| {
            this.0 *= na::Similarity2::from_scaling(factor);
            Ok(())
        });

        // methods.add_method_mut("set_matrix", |_ctx, this, matrix: Vec<f32>| {
        //     this.0 = na::Transform2::from_matrix_unchecked(na::Matrix3::from_column_slice(&matrix));
        //     Ok(())
        // });

        methods.add_method_mut(
            "set_transformation",
            |_ctx,
             this,
             (x, y, angle, sx, sy, ox, oy, kx, ky): (
                f32,
                f32,
                Option<f32>,
                Option<f32>,
                Option<f32>,
                Option<f32>,
                Option<f32>,
                Option<f32>,
                Option<f32>,
            )| {
                let angle = angle.unwrap_or(0.);
                let sx = sx.unwrap_or(1.);
                let sy = sy.unwrap_or(sx);
                let ox = ox.unwrap_or(0.);
                let oy = oy.unwrap_or(0.);
                let kx = kx.unwrap_or(0.);
                let ky = ky.unwrap_or(0.);

                let c = angle.cos();
                let s = angle.sin();

                let m = this.0.matrix_mut();
                *m = na::Matrix3::zeros();

                m[(0, 0)] = c * sx - ky * s * sy;
                m[(1, 0)] = s * sx + ky * c * sy;
                m[(0, 1)] = kx * c * sx - s * sy;
                m[(1, 1)] = kx * s * sx + c * sy;
                m[(0, 2)] = x - ox * m[(0, 0)] - oy * m[(0, 1)];
                m[(1, 2)] = y - ox * m[(1, 0)] - oy * m[(1, 1)];
                m[(2, 2)] = 1.;

                Ok(())
            },
        );

        methods.add_method_mut("shear", |_ctx, this, (kx, ky): (f32, f32)| {
            this.0 *= na::Transform2::from_matrix_unchecked(na::Matrix3::new(
                1., ky, 0., kx, 1., 0., 0., 0., 1.,
            ));
            Ok(())
        });

        methods.add_method_mut("transform_point", |_ctx, this, (x, y): (f32, f32)| {
            let p = this.0.transform_point(&na::Point2::new(x, y));
            Ok((p.x, p.y))
        });

        methods.add_method_mut("translate", |_ctx, this, (x, y): (f32, f32)| {
            this.0 *= na::Translation2::new(x, y);
            Ok(())
        });

        methods.add_meta_function(
            LuaMetaMethod::Mul,
            |_ctx, (a, b): (Transform, Transform)| Ok(Transform(a.0 * b.0)),
        );
    }
}

pub fn new_transform(_ctx: LuaContext, _: ()) -> LuaResult<Transform> {
    Ok(Transform(na::Transform2::identity()))
}

pub fn load<'lua>(lua: LuaContext<'lua>) -> Result<LuaValue<'lua>> {
    let table = lua.create_table_from(vec![
        ("Transform", lua.create_function(new_transform)?),
        ("sinh", lua.create_function(|_lua, f: f32| Ok(f.sinh()))?),
        ("cosh", lua.create_function(|_lua, f: f32| Ok(f.cosh()))?),
        ("tanh", lua.create_function(|_lua, f: f32| Ok(f.tanh()))?),
    ])?;

    Ok(LuaValue::Table(table))
}

inventory::submit! {
    crate::api::Module::parse("sludge.math", load)
}
