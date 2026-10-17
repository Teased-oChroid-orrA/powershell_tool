//! Raw STEP cylindrical parameter bounds: U in explicitly assigned radians,
//! V in the bound source length unit. No seam, trim or master inference.
use crate::{
    analytic_step::{call, scalar},
    geometry_bounds::Bounds3,
    native_interval::Interval,
    step::{Entity, EntityReference, Error, ErrorKind, Inventory, Value},
    step_body::BodyContext,
    step_frame::SourceFrame,
};
fn fail(e: &Entity, kind: ErrorKind, message: &str) -> Error {
    Error {
        offset: e.offset,
        kind,
        detail: message.into(),
    }
}
fn radian_unit(data: &Inventory, context: &Entity) -> Result<EntityReference, Error> {
    let components: Vec<_> = context
        .components
        .iter()
        .filter(|c| c.name == "GLOBAL_UNIT_ASSIGNED_CONTEXT")
        .collect();
    let [component] = components.as_slice() else {
        return Err(fail(
            context,
            ErrorKind::Units,
            "cylinder needs one assigned angular-unit context",
        ));
    };
    let [Value::List(units)] = component.args.as_slice() else {
        return Err(fail(
            context,
            ErrorKind::Units,
            "invalid assigned angular units",
        ));
    };
    let mut angular = None;
    for unit in units {
        let Value::Ref(id) = unit else {
            return Err(fail(
                context,
                ErrorKind::Units,
                "assigned unit must be a reference",
            ));
        };
        let e = &data.entities()[id];
        if !e.components.iter().any(|c| c.name == "PLANE_ANGLE_UNIT") {
            continue;
        }
        if angular.is_some() || e.components.len() != 3 {
            return Err(fail(
                e,
                ErrorKind::Units,
                "ambiguous or unsupported cylinder angular units",
            ));
        }
        let count = |name: &str| e.components.iter().filter(|c| c.name == name).count();
        if count("NAMED_UNIT") != 1 || count("PLANE_ANGLE_UNIT") != 1 || count("SI_UNIT") != 1 {
            return Err(fail(
                e,
                ErrorKind::Units,
                "cylinder requires explicit unprefixed SI radians",
            ));
        }
        for c in &e.components {
            let accepted = match (c.name.as_str(), c.args.as_slice()) {
                ("NAMED_UNIT", [Value::Derived]) => true,
                ("PLANE_ANGLE_UNIT", []) => true,
                ("SI_UNIT", [Value::Omitted, Value::Enum(name)]) => name == "RADIAN",
                _ => false,
            };
            if !accepted {
                return Err(fail(
                    e,
                    ErrorKind::Units,
                    "cylinder requires explicit unprefixed SI radians",
                ));
            }
        }
        angular = data.entity_reference(*id);
    }
    angular.ok_or_else(|| {
        fail(
            context,
            ErrorKind::Units,
            "no assigned plane-angle unit for cylinder",
        )
    })
}
#[derive(Debug)]
pub struct CylinderParameterMap {
    face: EntityReference,
    surface: EntityReference,
    context: EntityReference,
    angle_unit: EntityReference,
    frame: SourceFrame,
    radius_mm: Interval,
    scale_mm: f64,
    offset: usize,
}
impl CylinderParameterMap {
    pub fn read(
        context: &BodyContext,
        data: &Inventory,
        face: EntityReference,
    ) -> Result<Self, Error> {
        let assigned = data.resolve_reference(context.length_context())?;
        let entity = data.resolve_reference(face)?;
        let topology = context.body().faces.get(&face.entity).ok_or_else(|| {
            fail(
                entity,
                ErrorKind::Geometry,
                "source is not a face of this body",
            )
        })?;
        let surface = &data.entities()[&topology.surface];
        let (placement, radius) = match call(surface, "CYLINDRICAL_SURFACE")? {
            [Value::String(_), Value::Ref(frame), radius] => {
                (&data.entities()[frame], scalar(surface, radius)?)
            }
            _ => {
                return Err(fail(
                    surface,
                    ErrorKind::Geometry,
                    "requires actual STEP CYLINDRICAL_SURFACE",
                ))
            }
        };
        let angle_unit = radian_unit(data, assigned)?;
        let scale_mm = context.millimetres_per_source_unit();
        let radius_mm = Interval::point(radius) * Interval::point(scale_mm);
        if !radius_mm.finite() || radius_mm.lo <= 0. || radius_mm.hi > 1e30 {
            return Err(fail(
                surface,
                ErrorKind::Geometry,
                "cylinder radius is not positively bounded",
            ));
        }
        Ok(Self {
            face,
            surface: data.entity_reference(surface.id).unwrap(),
            context: context.length_context(),
            angle_unit,
            frame: SourceFrame::read(data, placement, scale_mm)?,
            radius_mm,
            scale_mm,
            offset: surface.offset,
        })
    }
    pub fn face_reference(&self) -> EntityReference {
        self.face
    }
    pub fn surface_reference(&self) -> EntityReference {
        self.surface
    }
    pub fn length_context(&self) -> EntityReference {
        self.context
    }
    pub fn angle_unit_reference(&self) -> EntityReference {
        self.angle_unit
    }
    pub fn millimetres_per_source_unit(&self) -> f64 {
        self.scale_mm
    }
    /// Bound all raw UV in the rectangle. U must lie in [-8,8] radians; wider
    /// ranges refuse explicitly instead of applying approximate periodic snaps.
    /// V alone scales by the assigned length factor. Bounds do not select trims.
    pub fn bounds(&self, uv: [[f64; 2]; 2]) -> Result<Bounds3, Error> {
        let error = |kind, message: &str| Error {
            offset: self.offset,
            kind,
            detail: message.into(),
        };
        if uv
            .iter()
            .any(|a| a.iter().any(|x| !x.is_finite()) || a[0] > a[1])
        {
            return Err(error(
                ErrorKind::Geometry,
                "invalid cylinder parameter rectangle",
            ));
        }
        let (sin, cos) = Interval::new(uv[0][0], uv[0][1])
            .sin_cos_bounded()
            .ok_or_else(|| {
                error(
                    ErrorKind::Geometry,
                    "cylinder angular domain outside certified [-8,8] radians",
                )
            })?;
        let axial = Interval::new(uv[1][0], uv[1][1]) * Interval::point(self.scale_mm);
        let world: [Interval; 3] = std::array::from_fn(|i| {
            self.frame.origin[i]
                + self.radius_mm * (self.frame.x[i] * cos + self.frame.y[i] * sin)
                + self.frame.z[i] * axial
        });
        if world
            .iter()
            .any(|x| !x.finite() || x.lo < -1e30 || x.hi > 1e30)
        {
            return Err(error(
                ErrorKind::Geometry,
                "cylinder mapping exceeds bounded world geometry",
            ));
        }
        Ok(Bounds3 {
            min: world.map(|x| x.lo),
            max: world.map(|x| x.hi),
        })
    }
    pub fn point_bounds(&self, uv: [f64; 2]) -> Result<Bounds3, Error> {
        self.bounds(uv.map(|x| [x, x]))
    }
}
