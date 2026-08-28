#!/usr/bin/env python3
"""Generate the original, asset-free Aris VRM prototype.

This is a reproducible Blender authoring script, not a downloader or a
derivative-asset extractor. It encodes the visual specification collected from
public references as new geometry and materials, then uses the official VRM
Add-on for Blender to export a VRM 1.0 file.

Example (run from the repository root):

    BLENDER=/path/to/Blender \
    VRM_ADDON_ROOT=/path/to/VRM-Addon-for-Blender \
    BLENDER_VRM_AUTOMATIC_LICENSE_CONFIRMATION=true \
    "$BLENDER" --background --python scripts/character/generate_aris_vrm.py -- \
      --out resources/characters/aris/model/aris-original.vrm \
      --blend resources/characters/aris/model/aris-original.blend \
      --render /tmp/aris-original-preview.png

The generated .vrm/.blend files are local development artifacts and are
ignored by Git. The model metadata deliberately says that it is an original
prototype and requires attribution; it does not claim to be an official game
asset.
"""

from __future__ import annotations

import argparse
import math
import os
import sys
from pathlib import Path
from typing import Sequence

import bpy
from mathutils import Vector


def _configure_addon() -> None:
    addon_root = os.environ.get("VRM_ADDON_ROOT")
    if addon_root:
        sys.path.insert(0, str(Path(addon_root) / "src"))

    import addon_utils  # type: ignore
    import io_scene_vrm  # type: ignore

    # Blender may have already enabled the add-on when this script is used from
    # an interactive file. Registering only once keeps the script repeatable.
    if not hasattr(bpy.types.Armature, "vrm_addon_extension"):
        addon_utils.enable("io_scene_vrm", default_set=False, persistent=False)
    if not hasattr(bpy.types.Armature, "vrm_addon_extension"):
        io_scene_vrm.register()

    # --factory-startup starts with an empty add-on preference collection. The
    # official add-on expects its preferences to exist during validation and
    # MToon setup, so create the standard Blender preference entry explicitly.
    if bpy.context.preferences.addons.get("io_scene_vrm") is None:
        addon = bpy.context.preferences.addons.new()
        addon.module = "io_scene_vrm"


_configure_addon()

from io_scene_vrm.common.ops import export_scene as export_ops  # type: ignore  # noqa: E402
from io_scene_vrm.common.ops import vrm as vrm_ops  # type: ignore  # noqa: E402
from io_scene_vrm.editor.extension_accessor import (  # type: ignore  # noqa: E402
    get_armature_extension,
    get_material_extension,
)
from io_scene_vrm.editor.mtoon1.property_group import (  # type: ignore  # noqa: E402
    convert_material_to_mtoon1,
)


Color = tuple[float, float, float, float]
Point = tuple[float, float, float]


class ModelParts:
    """Named objects used to attach geometry to the humanoid rig."""

    def __init__(self) -> None:
        self.by_bone: dict[str, list[bpy.types.Object]] = {}
        self.face: bpy.types.Object | None = None
        self.left_eye: bpy.types.Object | None = None
        self.right_eye: bpy.types.Object | None = None
        self.mouth: bpy.types.Object | None = None

    def add(self, obj: bpy.types.Object, bone: str) -> bpy.types.Object:
        self.by_bone.setdefault(bone, []).append(obj)
        return obj


def parse_args() -> argparse.Namespace:
    blender_args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=Path("resources/characters/aris/model/aris-original.vrm"),
    )
    parser.add_argument(
        "--blend",
        type=Path,
        default=Path("resources/characters/aris/model/aris-original.blend"),
    )
    parser.add_argument("--render", type=Path, default=Path("/tmp/aris-original-preview.png"))
    return parser.parse_args(blender_args)


def set_input(node: bpy.types.Node, name: str, value: object) -> None:
    socket = node.inputs.get(name)
    if socket is not None:
        socket.default_value = value  # type: ignore[assignment]


def make_material(
    name: str,
    color: Color,
    *,
    roughness: float = 0.58,
    metallic: float = 0.0,
    emission: Color | None = None,
    emission_strength: float = 0.0,
) -> bpy.types.Material:
    material = bpy.data.materials.new(name)
    material.use_nodes = True
    material.diffuse_color = color
    nodes = material.node_tree.nodes
    principled = nodes.get("Principled BSDF")
    if principled is not None:
        set_input(principled, "Base Color", color)
        set_input(principled, "Roughness", roughness)
        set_input(principled, "Metallic", metallic)
        if emission is not None:
            set_input(principled, "Emission Color", emission)
            set_input(principled, "Emission Strength", emission_strength)
    return material


def assign_material(obj: bpy.types.Object, surface: bpy.types.Material) -> bpy.types.Object:
    if obj.data and hasattr(obj.data, "materials"):
        obj.data.materials.append(surface)  # type: ignore[union-attr]
    return obj


def smooth(obj: bpy.types.Object) -> bpy.types.Object:
    if isinstance(obj.data, bpy.types.Mesh):
        for polygon in obj.data.polygons:
            polygon.use_smooth = True
    return obj


def apply_scale(obj: bpy.types.Object) -> bpy.types.Object:
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    obj.select_set(False)
    return obj


def add_uv_sphere(
    name: str,
    location: Point,
    scale: Point,
    surface: bpy.types.Material,
    *,
    segments: int = 40,
    rings: int = 24,
) -> bpy.types.Object:
    bpy.ops.mesh.primitive_uv_sphere_add(
        segments=segments,
        ring_count=rings,
        location=location,
    )
    obj = bpy.context.object
    assert obj is not None
    obj.name = name
    obj.scale = scale
    apply_scale(obj)
    assign_material(obj, surface)
    return smooth(obj)


def add_ico_sphere(
    name: str,
    location: Point,
    scale: Point,
    surface: bpy.types.Material,
    *,
    subdivisions: int = 3,
) -> bpy.types.Object:
    bpy.ops.mesh.primitive_ico_sphere_add(
        subdivisions=subdivisions,
        location=location,
    )
    obj = bpy.context.object
    assert obj is not None
    obj.name = name
    obj.scale = scale
    apply_scale(obj)
    assign_material(obj, surface)
    return smooth(obj)


def add_rounded_box(
    name: str,
    location: Point,
    dimensions: Point,
    surface: bpy.types.Material,
    *,
    bevel: float = 0.05,
) -> bpy.types.Object:
    bpy.ops.mesh.primitive_cube_add(location=location)
    obj = bpy.context.object
    assert obj is not None
    obj.name = name
    obj.dimensions = dimensions
    apply_scale(obj)
    modifier = obj.modifiers.new("soft_edges", "BEVEL")
    modifier.width = bevel
    modifier.segments = 4
    modifier.limit_method = "ANGLE"
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    bpy.ops.object.modifier_apply(modifier=modifier.name)
    obj.select_set(False)
    assign_material(obj, surface)
    return smooth(obj)


def add_cylinder(
    name: str,
    location: Point,
    radius: float,
    depth: float,
    surface: bpy.types.Material,
    *,
    vertices: int = 32,
) -> bpy.types.Object:
    bpy.ops.mesh.primitive_cylinder_add(
        vertices=vertices,
        radius=radius,
        depth=depth,
        location=location,
    )
    obj = bpy.context.object
    assert obj is not None
    obj.name = name
    assign_material(obj, surface)
    return smooth(obj)


def add_cone(
    name: str,
    location: Point,
    radius1: float,
    radius2: float,
    depth: float,
    surface: bpy.types.Material,
    *,
    vertices: int = 40,
) -> bpy.types.Object:
    bpy.ops.mesh.primitive_cone_add(
        vertices=vertices,
        radius1=radius1,
        radius2=radius2,
        depth=depth,
        location=location,
    )
    obj = bpy.context.object
    assert obj is not None
    obj.name = name
    assign_material(obj, surface)
    return smooth(obj)


def add_torus(
    name: str,
    location: Point,
    major_radius: float,
    minor_radius: float,
    surface: bpy.types.Material,
    *,
    rotation: Point = (0.0, 0.0, 0.0),
) -> bpy.types.Object:
    bpy.ops.mesh.primitive_torus_add(
        major_segments=48,
        minor_segments=12,
        location=location,
        major_radius=major_radius,
        minor_radius=minor_radius,
        rotation=rotation,
    )
    obj = bpy.context.object
    assert obj is not None
    obj.name = name
    assign_material(obj, surface)
    return smooth(obj)


def add_cylinder_between(
    name: str,
    start: Point,
    end: Point,
    radius: float,
    surface: bpy.types.Material,
    *,
    vertices: int = 28,
) -> bpy.types.Object:
    start_vec = Vector(start)
    end_vec = Vector(end)
    direction = end_vec - start_vec
    length = direction.length
    bpy.ops.mesh.primitive_cylinder_add(
        vertices=vertices,
        radius=radius,
        depth=length,
        location=(start_vec + end_vec) / 2,
    )
    obj = bpy.context.object
    assert obj is not None
    obj.name = name
    obj.rotation_mode = "QUATERNION"
    obj.rotation_quaternion = Vector((0.0, 0.0, 1.0)).rotation_difference(
        direction.normalized()
    )
    assign_material(obj, surface)
    return smooth(obj)


def add_curve_lock(
    name: str,
    points: Sequence[Point],
    radius: float,
    surface: bpy.types.Material,
) -> bpy.types.Object:
    curve = bpy.data.curves.new(name, "CURVE")
    curve.dimensions = "3D"
    curve.resolution_u = 10
    curve.bevel_depth = radius
    curve.bevel_resolution = 4
    spline = curve.splines.new("BEZIER")
    spline.bezier_points.add(len(points) - 1)
    for bezier, point in zip(spline.bezier_points, points):
        bezier.co = point
        bezier.handle_left_type = "AUTO"
        bezier.handle_right_type = "AUTO"
    obj = bpy.data.objects.new(name, curve)
    bpy.context.collection.objects.link(obj)
    assign_material(obj, surface)
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    bpy.ops.object.convert(target="MESH")
    obj.select_set(False)
    return smooth(obj)


def add_extruded_panel(
    name: str,
    points: Sequence[tuple[float, float]],
    z: float,
    depth: float,
    surface: bpy.types.Material,
    *,
    bevel: float = 0.018,
) -> bpy.types.Object:
    vertices = [(x, y, z) for x, y in points] + [(x, y, z - depth) for x, y in points]
    count = len(points)
    faces: list[tuple[int, ...]] = [
        tuple(range(count)),
        tuple(range(count, count * 2))[::-1],
    ]
    for index in range(count):
        next_index = (index + 1) % count
        faces.append((index, next_index, count + next_index, count + index))
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    assign_material(obj, surface)
    modifier = obj.modifiers.new("panel_soft_edges", "BEVEL")
    modifier.width = bevel
    modifier.segments = 3
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    bpy.ops.object.modifier_apply(modifier=modifier.name)
    obj.select_set(False)
    return smooth(obj)


def add_shape_key(
    obj: bpy.types.Object,
    name: str,
    transform,
) -> None:
    if not isinstance(obj.data, bpy.types.Mesh):
        return
    if obj.data.shape_keys is None:
        obj.shape_key_add(name="Basis")
    key = obj.shape_key_add(name=name)
    for index, vertex in enumerate(obj.data.vertices):
        key.data[index].co = transform(vertex.co.copy(), index)


def create_rig() -> bpy.types.Object:
    bpy.ops.object.armature_add(location=(0.0, 0.0, 0.0))
    armature = bpy.context.object
    assert armature is not None
    armature.name = "ArisOriginalArmature"
    armature.data.name = "ArisOriginalHumanoid"
    armature.show_in_front = True
    armature.display_type = "WIRE"

    bpy.ops.object.mode_set(mode="EDIT")
    edit_bones = armature.data.edit_bones
    edit_bones.remove(edit_bones[0])

    def add_bone(
        name: str,
        head: Point,
        tail: Point,
        parent: str | None = None,
    ) -> None:
        bone = edit_bones.new(name)
        bone.head = head
        bone.tail = tail
        bone.use_deform = True
        if parent:
            bone.parent = edit_bones.get(parent)
            bone.use_connect = False

    add_bone("hips", (0.0, 0.82, 0.0), (0.0, 1.04, 0.0))
    add_bone("spine", (0.0, 1.04, 0.0), (0.0, 1.46, 0.0), "hips")
    add_bone("chest", (0.0, 1.46, 0.0), (0.0, 1.86, 0.0), "spine")
    add_bone("neck", (0.0, 1.86, 0.0), (0.0, 2.12, 0.0), "chest")
    add_bone("head", (0.0, 2.12, 0.0), (0.0, 2.78, 0.0), "neck")
    add_bone("jaw", (0.0, 2.32, 0.36), (0.0, 2.17, 0.39), "head")
    add_bone("left_eye", (-0.18, 2.47, 0.42), (-0.18, 2.47, 0.55), "head")
    add_bone("right_eye", (0.18, 2.47, 0.42), (0.18, 2.47, 0.55), "head")

    for side, sign in (("left", -1.0), ("right", 1.0)):
        add_bone(
            f"{side}_upper_arm",
            (0.35 * sign, 1.78, 0.0),
            (0.68 * sign, 1.58, 0.0),
            "chest",
        )
        add_bone(
            f"{side}_lower_arm",
            (0.68 * sign, 1.58, 0.0),
            (0.86 * sign, 1.26, 0.0),
            f"{side}_upper_arm",
        )
        add_bone(
            f"{side}_hand",
            (0.86 * sign, 1.26, 0.0),
            (0.91 * sign, 1.15, 0.08),
            f"{side}_lower_arm",
        )
        add_bone(
            f"{side}_upper_leg",
            (0.18 * sign, 0.83, 0.0),
            (0.20 * sign, 0.47, 0.0),
            "hips",
        )
        add_bone(
            f"{side}_lower_leg",
            (0.20 * sign, 0.47, 0.0),
            (0.20 * sign, 0.12, 0.0),
            f"{side}_upper_leg",
        )
        add_bone(
            f"{side}_foot",
            (0.20 * sign, 0.12, 0.0),
            (0.20 * sign, 0.08, 0.20),
            f"{side}_lower_leg",
        )
        add_bone(
            f"{side}_toes",
            (0.20 * sign, 0.08, 0.20),
            (0.20 * sign, 0.08, 0.34),
            f"{side}_foot",
        )

    # Hair chains are separate deform bones, so the exported VRM can acquire
    # spring-bone motion without changing the desktop renderer contract.
    for side, sign in (("left", -1.0), ("right", 1.0)):
        add_bone(
            f"hair_{side}_1",
            (0.42 * sign, 2.40, -0.04),
            (0.56 * sign, 2.08, -0.10),
            "head",
        )
        add_bone(
            f"hair_{side}_2",
            (0.56 * sign, 2.08, -0.10),
            (0.62 * sign, 1.70, -0.14),
            f"hair_{side}_1",
        )
        add_bone(
            f"hair_{side}_3",
            (0.62 * sign, 1.70, -0.14),
            (0.57 * sign, 1.28, -0.12),
            f"hair_{side}_2",
        )

    bpy.ops.object.mode_set(mode="OBJECT")
    armature.hide_render = True
    armature.hide_set(True)
    return armature


def attach_to_rig(parts: ModelParts, armature: bpy.types.Object) -> None:
    bpy.context.view_layer.update()
    for bone_name, objects in parts.by_bone.items():
        pose_bone = armature.pose.bones.get(bone_name)
        if pose_bone is None:
            raise RuntimeError(f"Missing rig bone: {bone_name}")
        parent_matrix = armature.matrix_world @ pose_bone.matrix
        for obj in objects:
            if obj.parent is not None:
                obj.parent = None
            world_matrix = obj.matrix_world.copy()
            obj.parent = armature
            obj.parent_type = "BONE"
            obj.parent_bone = bone_name
            obj.matrix_parent_inverse = parent_matrix.inverted()
            obj.matrix_world = world_matrix


def bind_expression(
    expression: object,
    obj: bpy.types.Object,
    shape_key: str,
    weight: float = 1.0,
) -> None:
    bind = expression.morph_target_binds.add()  # type: ignore[attr-defined]
    bind.node.mesh_object_name = obj.name
    bind.index = shape_key
    bind.weight = weight


def setup_vrm_metadata(armature: bpy.types.Object, parts: ModelParts) -> None:
    extension = get_armature_extension(armature.data)
    extension.spec_version = "1.0"

    meta = extension.vrm1.meta
    meta.vrm_name = "Aris Original Desktop Companion Prototype"
    meta.version = "0.1.0"
    meta.copyright_information = (
        "Original prototype geometry; character concept references belong to their respective rights holders."
    )
    meta.contact_information = "desktop-ai-companion local prototype"
    meta.third_party_licenses = "No third-party character mesh, texture, voice, or game asset included."
    meta.avatar_permission = "onlyAuthor"
    meta.commercial_usage = "personalNonProfit"
    meta.credit_notation = "required"
    meta.allow_redistribution = False
    meta.modification = "allowModification"
    author = meta.authors.add()
    author.value = "desktop-ai-companion contributors"
    reference = meta.references.add()
    reference.value = "Public character design references; see docs/character/aris-model-research.md"

    humanoid = extension.vrm1.humanoid.human_bones
    required_mapping = {
        "hips": "hips",
        "spine": "spine",
        "chest": "chest",
        "neck": "neck",
        "head": "head",
        "jaw": "jaw",
        "left_eye": "left_eye",
        "right_eye": "right_eye",
        "left_upper_arm": "left_upper_arm",
        "left_lower_arm": "left_lower_arm",
        "left_hand": "left_hand",
        "right_upper_arm": "right_upper_arm",
        "right_lower_arm": "right_lower_arm",
        "right_hand": "right_hand",
        "left_upper_leg": "left_upper_leg",
        "left_lower_leg": "left_lower_leg",
        "left_foot": "left_foot",
        "left_toes": "left_toes",
        "right_upper_leg": "right_upper_leg",
        "right_lower_leg": "right_lower_leg",
        "right_foot": "right_foot",
        "right_toes": "right_toes",
    }
    for human_bone_name, bone_name in required_mapping.items():
        getattr(humanoid, human_bone_name).node.bone_name = bone_name
    extension.vrm1.humanoid.pose = "restPositionPose"

    if parts.left_eye is None or parts.right_eye is None or parts.mouth is None:
        raise RuntimeError("Expression meshes were not created")
    expressions = extension.vrm1.expressions.preset
    bind_expression(expressions.blink, parts.left_eye, "blink")
    bind_expression(expressions.blink, parts.right_eye, "blink")
    bind_expression(expressions.aa, parts.mouth, "aa")
    bind_expression(expressions.ih, parts.mouth, "aa", 0.72)
    bind_expression(expressions.ou, parts.mouth, "aa", 0.82)
    bind_expression(expressions.happy, parts.mouth, "happy")
    bind_expression(expressions.surprised, parts.mouth, "surprised")
    extension.vrm1.expressions.fill_missing_expression_names()

    # Configure a small amount of hair physics. The geometry is rigidly
    # attached to the chains; a later weighted retopology can reuse the same
    # named bones without changing the VRM metadata contract.
    spring_bone = extension.spring_bone1
    spring_bone.enable_animation = True
    spring = spring_bone.springs.add()
    spring.vrm_name = "Long hair"
    for side in ("left", "right"):
        for index in (1, 2, 3):
            joint = spring.joints.add()
            joint.node.bone_name = f"hair_{side}_{index}"
            joint.stiffness = 0.65 if index == 1 else 0.45
            joint.drag_force = 0.65
            joint.gravity_power = 0.08
            joint.gravity_dir = (0.0, -1.0, 0.0)
            joint.hit_radius = 0.06


def create_model() -> tuple[bpy.types.Object, ModelParts, list[bpy.types.Material]]:
    # Clean factory scene in case the script is run from a saved file.
    bpy.ops.object.mode_set(mode="OBJECT") if bpy.context.object and bpy.context.object.mode != "OBJECT" else None
    # Direct datablock removal also clears hidden/default objects that are not
    # selected in the background view layer (a factory Cube otherwise leaks
    # into the exported/rendered asset).
    for obj in list(bpy.data.objects):
        bpy.data.objects.remove(obj, do_unlink=True)
    for datablocks in (bpy.data.meshes, bpy.data.curves, bpy.data.materials, bpy.data.cameras, bpy.data.lights):
        for datablock in list(datablocks):
            if datablock.users == 0:
                datablocks.remove(datablock)

    materials = [
        make_material("Aris_Skin", (0.98, 0.76, 0.74, 1.0), roughness=0.74),
        make_material("Aris_Hair_Deep", (0.018, 0.032, 0.105, 1.0), roughness=0.42, metallic=0.05),
        make_material("Aris_Hair_Blue", (0.025, 0.15, 0.29, 1.0), roughness=0.38, metallic=0.08),
        make_material("Aris_Hair_Teal", (0.02, 0.62, 0.70, 1.0), roughness=0.30, metallic=0.12, emission=(0.0, 0.16, 0.20, 1.0), emission_strength=0.18),
        make_material("Aris_Coat", (0.86, 0.91, 0.98, 1.0), roughness=0.64),
        make_material("Aris_Coat_Shadow", (0.60, 0.68, 0.82, 1.0), roughness=0.68),
        make_material("Aris_Navy", (0.025, 0.045, 0.13, 1.0), roughness=0.48),
        make_material("Aris_Teal", (0.03, 0.68, 0.76, 1.0), roughness=0.32, metallic=0.14, emission=(0.0, 0.20, 0.24, 1.0), emission_strength=0.22),
        make_material("Aris_Gold", (0.98, 0.62, 0.17, 1.0), roughness=0.28, metallic=0.52, emission=(0.10, 0.04, 0.0, 1.0), emission_strength=0.12),
        make_material("Aris_Eye", (0.03, 0.53, 0.93, 1.0), roughness=0.17, metallic=0.10, emission=(0.0, 0.14, 0.24, 1.0), emission_strength=0.35),
        make_material("Aris_Eye_White", (0.95, 0.98, 1.0, 1.0), roughness=0.32),
        make_material("Aris_Mouth", (0.20, 0.035, 0.10, 1.0), roughness=0.44),
        make_material("Aris_Blush", (1.0, 0.22, 0.35, 0.58), roughness=0.72),
        make_material("Aris_Device", (0.015, 0.025, 0.08, 1.0), roughness=0.32, metallic=0.18),
    ]
    skin, hair_deep, hair_blue, hair_teal, coat, coat_shadow, navy, teal, gold, eye, eye_white, mouth_surface, blush, device = materials

    armature = create_rig()
    parts = ModelParts()

    # Legs, shoes and a shaped pleated skirt.
    parts.add(add_cylinder_between("left_leg", (-0.19, 0.18, 0.0), (-0.20, 0.78, 0.0), 0.115, skin), "left_lower_leg")
    parts.add(add_cylinder_between("right_leg", (0.19, 0.18, 0.0), (0.20, 0.78, 0.0), 0.115, skin), "right_lower_leg")
    parts.add(add_rounded_box("left_boot", (-0.20, 0.10, 0.10), (0.31, 0.20, 0.53), navy, bevel=0.06), "left_foot")
    parts.add(add_rounded_box("right_boot", (0.20, 0.10, 0.10), (0.31, 0.20, 0.53), navy, bevel=0.06), "right_foot")
    parts.add(add_cone("pleated_skirt", (0.0, 1.00, 0.0), 0.60, 0.44, 0.54, navy), "hips")
    parts.add(add_torus("skirt_hem", (0.0, 0.74, 0.0), 0.49, 0.028, teal, rotation=(math.pi / 2, 0.0, 0.0)), "hips")
    for index in range(12):
        angle = (math.tau * index) / 12.0
        radius = 0.515
        pleat = add_rounded_box(
            f"skirt_pleat_{index:02d}",
            (math.sin(angle) * radius, 1.01, math.cos(angle) * radius),
            (0.045, 0.44, 0.028),
            coat_shadow if index % 2 else navy,
            bevel=0.012,
        )
        pleat.rotation_euler[1] = angle
        parts.add(pleat, "hips")

    # Layered jacket and inner shirt.
    parts.add(add_rounded_box("inner_shirt", (0.0, 1.50, 0.02), (0.68, 0.70, 0.42), navy, bevel=0.15), "spine")
    parts.add(add_rounded_box("coat_back", (0.0, 1.57, -0.15), (0.92, 0.80, 0.24), coat, bevel=0.18), "spine")
    parts.add(add_rounded_box("coat_chest_overlay", (0.0, 1.68, 0.17), (0.76, 0.46, 0.12), coat, bevel=0.10), "chest")
    for sign, label in ((-1.0, "left"), (1.0, "right")):
        panel = add_rounded_box(f"{label}_coat_panel", (0.34 * sign, 1.55, 0.20), (0.30, 0.74, 0.14), coat_shadow, bevel=0.055)
        panel.rotation_euler[2] = 0.07 * sign
        parts.add(panel, "chest")
        trim = add_rounded_box(f"{label}_coat_trim", (0.22 * sign, 1.55, 0.285), (0.035, 0.66, 0.024), teal, bevel=0.012)
        trim.rotation_euler[2] = 0.07 * sign
        parts.add(trim, "chest")
        add_cylinder_between(f"{label}_sleeve", (0.39 * sign, 1.82, 0.02), (0.68 * sign, 1.31, 0.15), 0.145, coat)
        sleeve = bpy.context.object
        assert sleeve is not None
        parts.add(sleeve, f"{label}_lower_arm")
        parts.add(add_uv_sphere(f"{label}_hand", (0.70 * sign, 1.24, 0.18), (0.13, 0.14, 0.13), skin, segments=24, rings=16), f"{label}_hand")
        cuff = add_torus(f"{label}_cuff", (0.62 * sign, 1.39, 0.15), 0.13, 0.018, teal, rotation=(math.pi / 2, 0.0, 0.0))
        cuff.rotation_euler[2] = 0.42 * sign
        parts.add(cuff, f"{label}_lower_arm")
        shoulder = add_uv_sphere(
            f"{label}_shoulder_cap",
            (0.42 * sign, 1.82, 0.03),
            (0.21, 0.17, 0.17),
            coat,
            segments=28,
            rings=18,
        )
        parts.add(shoulder, "chest")

    collar = add_rounded_box("collar", (0.0, 1.91, 0.29), (0.34, 0.15, 0.09), coat, bevel=0.025)
    parts.add(collar, "neck")
    tie = add_cone("teal_tie", (0.0, 1.71, 0.335), 0.095, 0.042, 0.34, teal, vertices=4)
    tie.rotation_euler[2] = math.pi
    parts.add(tie, "chest")
    parts.add(add_uv_sphere("tie_clip", (0.0, 1.83, 0.37), (0.052, 0.052, 0.035), gold, segments=20, rings=12), "chest")
    for sign, label in ((-1.0, "left"), (1.0, "right")):
        for y in (1.53, 1.70):
            parts.add(add_uv_sphere(f"{label}_coat_button_{y}", (0.31 * sign, y, 0.29), (0.026, 0.026, 0.018), gold, segments=16, rings=10), "chest")

    # Handheld game device with screen, d-pad and glowing buttons.
    console = add_rounded_box("handheld_console", (-0.49, 1.28, 0.28), (0.32, 0.22, 0.065), device, bevel=0.028)
    console.rotation_euler[2] = -0.16
    parts.add(console, "left_hand")
    screen = add_rounded_box("handheld_screen", (-0.49, 1.30, 0.322), (0.19, 0.115, 0.012), teal, bevel=0.012)
    screen.rotation_euler[2] = -0.16
    parts.add(screen, "left_hand")
    for index, (x, y) in enumerate(((-0.58, 1.23), (-0.41, 1.23), (-0.49, 1.20))):
        button = add_uv_sphere(f"console_button_{index}", (x, y, 0.34), (0.018, 0.018, 0.012), gold if index == 2 else teal, segments=12, rings=8)
        button.rotation_euler[2] = -0.16
        parts.add(button, "left_hand")

    # Face, eyes, mouth and small expression-friendly details.
    face = add_uv_sphere("ArisFace", (0.0, 2.39, 0.06), (0.45, 0.53, 0.42), skin, segments=52, rings=36)
    parts.face = face
    parts.add(face, "head")
    parts.add(add_cylinder("neck_mesh", (0.0, 2.00, 0.0), 0.15, 0.22, skin), "neck")
    for sign, label in ((-1.0, "left"), (1.0, "right")):
        eye_white_obj = add_uv_sphere(f"{label}_eye_white", (0.17 * sign, 2.50, 0.48), (0.120, 0.155, 0.055), eye_white, segments=36, rings=24)
        parts.add(eye_white_obj, "head")
        iris = add_uv_sphere(f"{label}_iris", (0.17 * sign, 2.49, 0.535), (0.075, 0.105, 0.032), eye, segments=32, rings=20)
        parts.add(iris, "head")
        iris_ring = add_torus(f"{label}_iris_ring", (0.17 * sign, 2.49, 0.558), 0.070, 0.010, teal)
        parts.add(iris_ring, "head")
        pupil = add_uv_sphere(f"{label}_pupil", (0.17 * sign, 2.49, 0.566), (0.032, 0.066, 0.014), navy, segments=20, rings=12)
        parts.add(pupil, "head")
        highlight = add_uv_sphere(f"{label}_eye_highlight", (0.145 * sign, 2.55, 0.584), (0.022, 0.035, 0.010), eye_white, segments=16, rings=10)
        parts.add(highlight, "head")
        lash = add_curve_lock(
            f"{label}_upper_lash",
            [(0.07 * sign, 2.63, 0.57), (0.17 * sign, 2.68, 0.58), (0.28 * sign, 2.62, 0.56)],
            0.018,
            hair_deep,
        )
        parts.add(lash, "head")
        brow = add_rounded_box(f"{label}_brow", (0.17 * sign, 2.75, 0.48), (0.16, 0.028, 0.024), hair_blue, bevel=0.012)
        brow.rotation_euler[2] = 0.08 * sign
        parts.add(brow, "head")

    mouth = add_uv_sphere("ArisMouth", (0.0, 2.19, 0.505), (0.072, 0.034, 0.025), mouth_surface, segments=24, rings=16)
    parts.mouth = mouth
    parts.add(mouth, "jaw")
    nose = add_uv_sphere("ArisNose", (0.0, 2.31, 0.505), (0.034, 0.045, 0.024), skin, segments=18, rings=12)
    parts.add(nose, "head")
    for sign, label in ((-1.0, "left"), (1.0, "right")):
        cheek = add_uv_sphere(f"{label}_cheek", (0.34 * sign, 2.26, 0.47), (0.080, 0.042, 0.010), blush, segments=20, rings=12)
        parts.add(cheek, "head")

    # The visible eye-white meshes are intentionally driven by the same key;
    # the dedicated objects are found by name so only their vertical aperture
    # changes during blink.
    left_eye = next(obj for obj in parts.by_bone["head"] if obj.name == "left_eye_white")
    right_eye = next(obj for obj in parts.by_bone["head"] if obj.name == "right_eye_white")
    add_shape_key(left_eye, "blink", lambda co, _index: Vector((co.x, co.y * 0.08, co.z)))
    add_shape_key(right_eye, "blink", lambda co, _index: Vector((co.x, co.y * 0.08, co.z)))
    add_shape_key(mouth, "aa", lambda co, _index: Vector((co.x * 1.12, co.y * 2.8, co.z)))
    add_shape_key(mouth, "happy", lambda co, _index: Vector((co.x, co.y * 0.45 + abs(co.x) * 0.16, co.z)))
    add_shape_key(mouth, "surprised", lambda co, _index: Vector((co.x * 0.82, co.y * 2.6, co.z)))
    parts.left_eye = left_eye
    parts.right_eye = right_eye

    # Hair cap, layered bangs and long side locks.
    cap = add_uv_sphere("hair_cap", (0.0, 2.63, -0.15), (0.53, 0.49, 0.43), hair_deep, segments=52, rings=36)
    parts.add(cap, "head")
    bang_panels = [
        ([(-0.53, 2.78), (-0.37, 2.98), (-0.14, 3.03), (-0.08, 2.55), (-0.23, 2.34), (-0.36, 2.59)], hair_blue),
        ([(-0.20, 2.98), (-0.02, 3.08), (0.14, 3.00), (0.09, 2.55), (0.01, 2.36), (-0.08, 2.60)], hair_deep),
        ([(0.06, 3.03), (0.28, 3.00), (0.52, 2.86), (0.38, 2.48), (0.21, 2.35), (0.12, 2.60)], hair_blue),
    ]
    for index, (points, surface) in enumerate(bang_panels):
        panel = add_extruded_panel(f"bang_panel_{index}", points, 0.50, 0.060, surface, bevel=0.028)
        parts.add(panel, "head")
    # Thin raised highlight locks break up the broad bang panels in close-up
    # views while keeping the silhouette and the VRM asset fully original.
    for index, sign in enumerate((-1.0, 1.0)):
        highlight_lock = add_curve_lock(
            f"bang_highlight_{index}",
            [(0.12 * sign, 2.96, 0.57), (0.22 * sign, 2.78, 0.58), (0.27 * sign, 2.54, 0.57)],
            0.014,
            hair_teal,
        )
        parts.add(highlight_lock, "head")
    for sign, label in ((-1.0, "left"), (1.0, "right")):
        side_lock = add_extruded_panel(
            f"{label}_side_lock",
            [(0.45 * sign, 2.69), (0.65 * sign, 2.42), (0.70 * sign, 1.75), (0.57 * sign, 1.30), (0.41 * sign, 1.52), (0.36 * sign, 2.25)],
            0.20,
            0.09,
            hair_blue if sign > 0 else hair_deep,
            bevel=0.030,
        )
        parts.add(side_lock, f"hair_{label}_1")
        tip = add_extruded_panel(
            f"{label}_side_lock_tip",
            [(0.56 * sign, 1.62), (0.70 * sign, 1.30), (0.57 * sign, 1.05), (0.42 * sign, 1.32)],
            0.22,
            0.065,
            hair_teal,
            bevel=0.024,
        )
        parts.add(tip, f"hair_{label}_2")
        second_lock = add_curve_lock(
            f"{label}_side_lock_secondary",
            [(0.50 * sign, 2.50, 0.13), (0.72 * sign, 2.17, 0.10), (0.74 * sign, 1.70, 0.06), (0.62 * sign, 1.38, 0.05)],
            0.050,
            hair_deep if sign > 0 else hair_blue,
        )
        parts.add(second_lock, f"hair_{label}_1")
        # High side ponytail.
        ponytail = add_curve_lock(
            f"{label}_ponytail",
            [(0.46 * sign, 2.62, -0.10), (0.70 * sign, 2.54, -0.14), (0.89 * sign, 2.24, -0.16), (0.82 * sign, 1.72, -0.12)],
            0.15,
            hair_deep if sign < 0 else hair_blue,
        )
        parts.add(ponytail, f"hair_{label}_1")
        pony_tip = add_curve_lock(
            f"{label}_ponytail_tip",
            [(0.83 * sign, 1.86, -0.13), (0.89 * sign, 1.60, -0.12), (0.78 * sign, 1.36, -0.10)],
            0.10,
            hair_teal,
        )
        parts.add(pony_tip, f"hair_{label}_2")
        band = add_torus(f"{label}_ponytail_band", (0.49 * sign, 2.55, -0.10), 0.14, 0.026, teal, rotation=(math.pi / 2, 0.0, 0.0))
        parts.add(band, "head")

    # Halo with layered rings and markers.
    halo = add_torus("halo_outer", (0.0, 2.65, -0.34), 0.73, 0.038, teal)
    parts.add(halo, "head")
    halo_inner = add_torus("halo_inner", (0.0, 2.65, -0.35), 0.50, 0.014, gold)
    parts.add(halo_inner, "head")
    for index, (x, y, rotation) in enumerate(((-0.61, 2.79, -0.25), (0.61, 2.79, 0.25), (0.0, 3.36, 0.0))):
        marker = add_rounded_box(f"halo_marker_{index}", (x, y, -0.34), (0.09, 0.22, 0.04), teal, bevel=0.02)
        marker.rotation_euler[2] = rotation
        parts.add(marker, "head")

    # Supernova-inspired back prop, deliberately authored as new compact parts.
    prop_body = add_rounded_box("back_game_prop", (0.62, 1.63, -0.28), (0.23, 0.88, 0.22), coat_shadow, bevel=0.06)
    prop_body.rotation_euler[2] = -0.28
    parts.add(prop_body, "spine")
    prop_core = add_rounded_box("back_game_prop_core", (0.62, 1.63, -0.15), (0.08, 0.65, 0.06), teal, bevel=0.018)
    prop_core.rotation_euler[2] = -0.28
    parts.add(prop_core, "spine")
    prop_end = add_cone("back_game_prop_end", (0.75, 2.13, -0.26), 0.15, 0.04, 0.25, gold, vertices=6)
    prop_end.rotation_euler[2] = math.pi - 0.28
    parts.add(prop_end, "spine")

    attach_to_rig(parts, armature)
    setup_vrm_metadata(armature, parts)
    return armature, parts, materials


def setup_render(render_path: Path) -> None:
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_EEVEE_NEXT"
    scene.render.resolution_x = 600
    scene.render.resolution_y = 760
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "PNG"
    scene.render.filepath = str(render_path)
    scene.render.film_transparent = False
    scene.world.color = (0.012, 0.020, 0.055)

    bpy.ops.object.camera_add(location=(0.0, 1.72, 7.6))
    camera = bpy.context.object
    assert camera is not None
    camera.name = "PreviewCamera"
    camera.data.lens = 62
    camera.rotation_euler = (Vector((0.0, 1.75, 0.0)) - camera.location).to_track_quat("-Z", "Y").to_euler()
    scene.camera = camera

    def area_light(name: str, location: Point, energy: float, size: float, color: Color) -> bpy.types.Object:
        bpy.ops.object.light_add(type="AREA", location=location)
        light = bpy.context.object
        assert light is not None
        light.name = name
        light.data.energy = energy
        light.data.shape = "DISK"
        light.data.size = size
        light.data.color = color[:3]
        light.rotation_euler = (Vector((0.0, 1.7, 0.0)) - light.location).to_track_quat("-Z", "Y").to_euler()
        return light

    area_light("KeyLight", (3.6, 5.0, 5.2), 900.0, 4.0, (0.82, 0.92, 1.0, 1.0))
    area_light("FillLight", (-4.0, 2.4, 3.5), 650.0, 3.0, (0.34, 0.72, 1.0, 1.0))
    area_light("RimLight", (0.0, 2.5, -3.5), 1150.0, 2.5, (0.20, 0.95, 1.0, 1.0))
    bpy.ops.mesh.primitive_plane_add(
        size=20,
        location=(0.0, 0.0, 0.0),
        rotation=(math.pi / 2, 0.0, 0.0),
    )
    floor = bpy.context.object
    assert floor is not None
    floor.name = "PreviewFloor"
    floor_material = make_material("PreviewFloorMaterial", (0.018, 0.028, 0.07, 1.0), roughness=0.88)
    assign_material(floor, floor_material)
    floor.hide_viewport = True
    bpy.ops.render.render(write_still=True)


def main() -> None:
    args = parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.blend.parent.mkdir(parents=True, exist_ok=True)
    args.render.parent.mkdir(parents=True, exist_ok=True)

    armature, parts, materials = create_model()

    # Convert ordinary authored materials to VRM MToon 1.0. If a future
    # Blender/Add-on combination changes this optional conversion, the script
    # fails loudly instead of silently exporting an unknown material setup.
    for surface in materials:
        convert_material_to_mtoon1(surface, bpy.context)
        mtoon = get_material_extension(surface).mtoon1
        mtoon.extensions.vrmc_materials_mtoon.shade_color_factor = (0.72, 0.78, 0.92)
        mtoon.extensions.vrmc_materials_mtoon.outline_width_mode = "worldCoordinates"
        mtoon.extensions.vrmc_materials_mtoon.outline_width_factor = 0.006
        mtoon.extensions.vrmc_materials_mtoon.outline_color_factor = (0.012, 0.025, 0.075)

    validation = vrm_ops.model_validate()
    if validation != {"FINISHED"}:
        raise RuntimeError(f"VRM validation failed: {validation}")

    export_result = export_ops.vrm(
        filepath=str(args.out),
        armature_object_name=armature.name,
        ignore_warning=True,
        export_invisibles=False,
        export_lights=False,
        export_gltf_animations=False,
    )
    if export_result != {"FINISHED"}:
        raise RuntimeError(f"VRM export failed: {export_result}")

    setup_render(args.render)
    bpy.ops.wm.save_as_mainfile(filepath=str(args.blend))
    print(f"Generated VRM: {args.out}")
    print(f"Generated Blend: {args.blend}")
    print(f"Generated Render: {args.render}")


if __name__ == "__main__":
    main()
