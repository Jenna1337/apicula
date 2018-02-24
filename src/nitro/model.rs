use cgmath::{Matrix3, Matrix4, One, vec3};
use errors::Result;
use nitro::info_block;
use nitro::Name;
use nitro::TextureParameters;
use util::{bits::BitField, cur::Cur, fixed::{fix16, fix32}};

pub struct Model {
    pub name: Name,
    pub materials: Vec<Material>,
    pub meshes: Vec<Mesh>,
    pub objects: Vec<Object>,
    pub inv_binds: Vec<Matrix4<f64>>,
    pub render_cmds: Vec<u8>,
    pub up_scale: f64,
    pub down_scale: f64,
}

pub struct Material {
    pub name: Name,
    pub texture_name: Option<Name>,
    pub palette_name: Option<Name>,
    pub params: TextureParameters,
    pub width: u16,
    pub height: u16,
    pub texture_mat: Matrix4<f64>,
}

pub struct Mesh {
    pub name: Name,
    pub commands: Vec<u8>,
}

pub struct Object {
    pub name: Name,
    pub trans: [f64; 3],
    pub rotation: Matrix3<f64>,
    pub scale: [f64; 3],
    pub xform: Matrix4<f64>,
}

pub fn read_model(cur: Cur, name: Name) -> Result<Model> {
    debug!("model: {:?}", name);
    debug!("=======================");

    fields!(cur, model {
        section_size: u32,
        render_cmds_off: u32,
        materials_off: u32,
        mesh_off: u32,
        inv_binds_off: u32,
        unknown1: [u8; 3],
        num_objects: u8, //TODO maybe this is the number of inv bind mats?
        num_materials: u8,
        num_meshes: u8,
        unknown2: [u8; 2],
        up_scale: (fix32(1,19,12)),
        down_scale: (fix32(1,19,12)),
        num_verts: u16,
        num_surfs: u16,
        num_tris: u16,
        num_quads: u16,
        bounding_box_x_min: (fix16(1,3,12)),
        bounding_box_y_min: (fix16(1,3,12)),
        bounding_box_z_min: (fix16(1,3,12)),
        bounding_box_x_max: (fix16(1,3,12)),
        bounding_box_y_max: (fix16(1,3,12)),
        bounding_box_z_max: (fix16(1,3,12)),
        unknown3: [u8; 8],
        objects_cur: Cur,
    });

    let meshes = read_meshes(cur + mesh_off)?;
    let objects = read_objects(objects_cur)?;
    let materials = read_materials(cur + materials_off)?;
    let render_cmds = read_render_cmds(cur + render_cmds_off)?;
    let inv_binds = read_inv_binds(cur + inv_binds_off, num_objects as usize);

    Ok(Model {
        name, materials, meshes, objects, inv_binds,
        render_cmds, up_scale, down_scale,
    })
}

// Meshes
//////////

fn read_meshes(cur: Cur) -> Result<Vec<Mesh>> {
    info_block::read::<u32>(cur)?
        .map(|(off, name)| read_mesh(cur + off, name))
        .collect()
}

fn read_mesh(cur: Cur, name: Name) -> Result<Mesh> {
    debug!("mesh: {:?}", name);

    fields!(cur, mesh {
        dummy: u16,
        section_size: u16,
        unknown: u32,
        commands_off: u32,
        commands_len: u32,
    });

    check!(section_size == 16)?;
    check!(commands_len % 4 == 0)?;

    let commands = (cur + commands_off)
        .next_n_u8s(commands_len as usize)?
        .to_vec();

    Ok(Mesh { name, commands })
}

// Materials
//////////////

fn read_materials(cur: Cur) -> Result<Vec<Material>> {
    fields!(cur, materials {
        texture_pairing_off: u16,
        palette_pairing_off: u16,
        end: Cur,
    });

    let mut materials = info_block::read::<u32>(end)?
        .map(|(off, name)| read_material(cur + off, name))
        .collect::<Result<Vec<_>>>()?;

    // Pair each texture with materials.
    let tex_cur = cur + texture_pairing_off;
    for ((off, num, _), name) in info_block::read::<(u16, u8, u8)>(tex_cur)? {
        trace!("texture pairing: {}", name);
        fields!(cur + off, texture_pairings {
            material_ids: [u8; num],
        });
        for &mat_id in material_ids {
            materials[mat_id as usize].texture_name = Some(name);
        }
    }

    // Pair each palette with materials.
    let pal_cur = cur + palette_pairing_off;
    for ((off, num, _), name) in info_block::read::<(u16, u8, u8)>(pal_cur)? {
        trace!("palette pairing: {}", name);
        fields!(cur + off, palette_pairings {
            material_ids: [u8; num],
        });
        for &mat_id in material_ids {
            materials[mat_id as usize].palette_name = Some(name);
        }
    }

    Ok(materials)
}

fn read_material(cur: Cur, name: Name) -> Result<Material> {
    debug!("material: {:?}", name);

    fields!(cur, material {
        dummy: u16,
        section_size: u16,
        dif_amb: u32,
        spe_emi: u32,
        polygon_attr: u32,
        unknown2: u32,
        params: u32,
        unknown3: u32,
        unknown4: u32, // flag for texture matrix?
        width: u16,
        height: u16,
        unknown5: (fix32(1,19,12)), // always 1?
        unknown6: (fix32(1,19,12)), // always 1?
        end: Cur,
    });

    let params = TextureParameters::from_u32(params);

    // For now, use the section size to determine whether there
    // is texture matrix data.
    let texture_mat = match section_size {
        60 => {
            fields!(end, texcoord_matrix {
                a: (fix32(1,19,12)),
                b: (fix32(1,19,12)),
            });
            Matrix4::from_nonuniform_scale(a, b, 1.0)
        }
        _ => Matrix4::from_scale(1.0),
    };

    Ok(Material {
        name,
        texture_name: None,
        palette_name: None,
        params,
        width,
        height,
        texture_mat,
    })
}

// Objects
////////////

fn read_objects(cur: Cur) -> Result<Vec<Object>> {
    info_block::read::<u32>(cur)?
        .map(|(off, name)| read_object(cur + off, name))
        .collect()
}

fn read_object(mut cur: Cur, name: Name) -> Result<Object> {
    let fx16 = |x| fix16(x, 1, 3, 12);
    let fx32 = |x| fix32(x, 1, 19, 12);

    trace!("object: {}", name);

    let flags = cur.next::<u16>()?;
    let t = flags.bits(0,1);
    let r = flags.bits(1,2);
    let s = flags.bits(2,3);
    let p = flags.bits(3,4);
    trace!("t={}, r={}, s={}, p={}", t, r, s, p);

    // Why in God's name is this here instead of with the
    // other rotation stuff?
    let m0 = cur.next::<u16>()?;

    let mut trans = [0.0, 0.0, 0.0];
    let mut rotation = Matrix3::one();
    let mut scale = [1.0, 1.0, 1.0];

    // Translation
    if t == 0 {
        let xyz = cur.next_n::<u32>(3)?;
        trans = [fx32(xyz.get(0)), fx32(xyz.get(1)), fx32(xyz.get(2))];
    }
    trace!("trans: {:?}", trans);

    // 3x3 Matrix (typically a rotation)
    if p == 1 {
        let a = fx16(cur.next::<u16>()?);
        let b = fx16(cur.next::<u16>()?);
        let select = flags.bits(4,8);
        let neg = flags.bits(8,12);
        use nitro::rotation::pivot_mat;
        rotation = pivot_mat(select, neg, a, b);
    } else if r == 0 {
        let m = cur.next_n::<u16>(8)?;
        rotation = Matrix3::new(
            fx16(m0), fx16(m.get(0)), fx16(m.get(1)),
            fx16(m.get(2)), fx16(m.get(3)), fx16(m.get(4)),
            fx16(m.get(5)), fx16(m.get(6)), fx16(m.get(7)),
        );
    }
    trace!("rotation: {:?}", rotation);

    // Scale
    if s == 0 {
        let xyz = cur.next_n::<u32>(3)?;
        scale = [fx32(xyz.get(0)), fx32(xyz.get(1)), fx32(xyz.get(2))];
    }
    trace!("scale: {:?}", scale);

    let xform =
        Matrix4::from_translation(vec3(trans[0], trans[1], trans[2])) *
        Matrix4::from(rotation) *
        Matrix4::from_nonuniform_scale(scale[0], scale[1], scale[2]);

    Ok(Object { name, trans, rotation, scale, xform })
}


// Inverse bind matrices
//////////////////////////

fn read_inv_binds(mut cur: Cur, num_objects: usize) -> Vec<Matrix4<f64>> {
    // Each element in the inv bind array consists of
    // * one 4 x 3 matrix, the inverse of some local-to-world object
    //  transform; this is the one we care about
    // * one 3 x 3 matrix, possibly for normals(?) that we ignore
    // Each matrix entry is a 4-byte fixed point number.
    let elem_size = (4*3 + 3*3) * 4;

    let mut inv_binds = Vec::<Matrix4<f64>>::with_capacity(num_objects);
    for _ in 0..num_objects {
        // Sometimes there aren't num_objects matrices, so just take as many as we
        // can. This doesn't matter since they might not even be used.
        // (This is why this function doesn't return a Result.)
        if cur.bytes_remaining() < elem_size { break; }

        let entries = cur.next_n::<u32>(4*3).unwrap();
        let m = |i| fix32(entries.get(i), 1, 19, 12);
        inv_binds.push(Matrix4::new(
            m(0), m(1), m(2), 0.0,
            m(3), m(4), m(5), 0.0,
            m(6), m(7), m(8), 0.0,
            m(9), m(10), m(11), 1.0,
        ));
        cur.jump_forward(3*3*4);
    }
    inv_binds
}

// Render commands
////////////////////

pub fn read_render_cmds(mut cur: Cur) -> Result<Vec<u8>> {
    use nitro::render_cmds::find_end;
    let end = find_end(cur)?;
    let len = end.pos() - cur.pos();
    Ok(cur.next_n_u8s(len).unwrap().to_vec())
}