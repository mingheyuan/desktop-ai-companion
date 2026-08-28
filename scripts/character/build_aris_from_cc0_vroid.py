#!/usr/bin/env python3
"""Build a higher-detail Aris study from an explicitly CC0 VRoid sample.

This script does not download a model. The caller must provide a local VRM
whose source and license have already been checked. It preserves the base
model's face/body detail, applies an Aris-inspired palette, and adds original
halo, uniform trim, skirt accents, tie, and handheld-device geometry.

The generated asset remains a local development artifact. See
resources/characters/aris/README.md for provenance and redistribution rules.
"""

from __future__ import annotations

import argparse
import colorsys
import math
import os
import sys
from pathlib import Path

import bpy
import bmesh
from mathutils import Vector


REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "scripts" / "character"))

# Reuse the tested authoring primitives and official add-on setup from the
# asset-free generator. The imported base remains a separate, license-scoped
# input and is never fetched by this script.
import generate_aris_vrm as authored  # type: ignore  # noqa: E402

from io_scene_vrm.common.ops import export_scene as export_ops  # type: ignore  # noqa: E402
from io_scene_vrm.editor.extension_accessor import (  # type: ignore  # noqa: E402
    get_armature_extension,
    get_material_extension,
)


Point = tuple[float, float, float]
Color = tuple[float, float, float, float]


def parse_args() -> argparse.Namespace:
    blender_args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--blend", type=Path, required=True)
    parser.add_argument("--render", type=Path, required=True)
    parser.add_argument("--hair-mode", choices=("base", "custom"), default="base")
    return parser.parse_args(blender_args)


def clear_scene() -> None:
    if bpy.context.object and bpy.context.object.mode != "OBJECT":
        bpy.ops.object.mode_set(mode="OBJECT")
    for obj in list(bpy.data.objects):
        bpy.data.objects.remove(obj, do_unlink=True)
    for datablocks in (
        bpy.data.meshes,
        bpy.data.curves,
        bpy.data.materials,
        bpy.data.cameras,
        bpy.data.lights,
    ):
        for datablock in list(datablocks):
            if datablock.users == 0:
                datablocks.remove(datablock)


def toon_material(name: str, color: Color, *, metallic: float = 0.0) -> bpy.types.Material:
    material = authored.make_material(name, color, roughness=0.54, metallic=metallic)
    authored.convert_material_to_mtoon1(material, bpy.context)
    toon = get_material_extension(material).mtoon1.extensions.vrmc_materials_mtoon
    toon.shade_color_factor = tuple(max(0.0, min(1.0, channel * 0.78)) for channel in color[:3])
    toon.outline_width_mode = "worldCoordinates"
    toon.outline_width_factor = 0.004
    toon.outline_color_factor = (0.015, 0.025, 0.07)
    return material


def parent_to_bone(obj: bpy.types.Object, armature: bpy.types.Object, bone_name: str) -> None:
    bone = armature.pose.bones.get(bone_name)
    if bone is None:
        raise RuntimeError(f"Base VRM is missing required bone: {bone_name}")
    world_matrix = obj.matrix_world.copy()
    obj.parent = armature
    obj.parent_type = "BONE"
    obj.parent_bone = bone_name
    obj.matrix_parent_inverse = (armature.matrix_world @ bone.matrix).inverted()
    obj.matrix_world = world_matrix


def add_extruded_xz(
    name: str,
    points: list[tuple[float, float]],
    y: float,
    depth: float,
    surface: bpy.types.Material,
    *,
    bevel: float = 0.012,
) -> bpy.types.Object:
    """Create an original beveled hair sheet in the z-up base coordinates."""
    vertices = [(x, y, z) for x, z in points] + [(x, y + depth, z) for x, z in points]
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
    authored.assign_material(obj, surface)
    modifier = obj.modifiers.new("hair_sheet_soft_edges", "BEVEL")
    modifier.width = bevel
    modifier.segments = 3
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    bpy.ops.object.modifier_apply(modifier=modifier.name)
    obj.select_set(False)
    return authored.smooth(obj)


def replace_slot(mesh: bpy.types.Mesh, slot_index: int, material: bpy.types.Material) -> None:
    while len(mesh.materials) <= slot_index:
        mesh.materials.append(None)
    mesh.materials[slot_index] = material


def tint_hair_texture(material: bpy.types.Material, hue: float) -> None:
    """Shift the CC0 hair texture toward blue while preserving highlights."""
    if not material.node_tree:
        return
    for node in material.node_tree.nodes:
        if node.bl_idname != "ShaderNodeTexImage" or node.image is None:
            continue
        if "BaseColorTexture" not in node.name:
            continue
        source = node.image
        tinted = source.copy()
        tinted.name = f"{source.name}_{material.name}_blue_tint"
        pixels = list(source.pixels)
        for index in range(0, len(pixels), 4):
            red, green, blue = pixels[index : index + 3]
            value = max(red, green, blue)
            saturation = min(1.0, max(0.38, max(red, green, blue) - min(red, green, blue) + 0.20))
            red, green, blue = colorsys.hsv_to_rgb(hue, saturation, value)
            pixels[index : index + 3] = (red, green, blue)
        tinted.pixels = pixels
        tinted.update()
        node.image = tinted


def clone_tinted_hair_material(source: bpy.types.Material, name: str, hue: float, shade: tuple[float, float, float]) -> bpy.types.Material:
    material = source.copy()
    material.name = name
    tint_hair_texture(material, hue)
    toon = get_material_extension(material).mtoon1.extensions.vrmc_materials_mtoon
    toon.shade_color_factor = shade
    toon.outline_width_mode = "worldCoordinates"
    toon.outline_width_factor = 0.0035
    toon.outline_color_factor = (0.01, 0.02, 0.06)
    return material


def repair_base_hair(hair: bpy.types.Object, deep_hair: bpy.types.Material, blue_hair: bpy.types.Material) -> None:
    """Keep detailed CC0 twin-tail geometry, remove only pointed ear faces."""
    mesh = hair.data
    assert isinstance(mesh, bpy.types.Mesh)
    edit_mesh = bmesh.new()
    edit_mesh.from_mesh(mesh)
    ear_faces = [
        face
        for face in edit_mesh.faces
        if face.calc_center_median().z > 1.55 and abs(face.calc_center_median().x) > 0.07
    ]
    if ear_faces:
        bmesh.ops.delete(edit_mesh, geom=ear_faces, context="FACES")
    edit_mesh.to_mesh(mesh)
    edit_mesh.free()
    replace_slot(mesh, 0, deep_hair)
    replace_slot(mesh, 1, blue_hair)


def recolor_base(armature: bpy.types.Object, hair_mode: str) -> tuple[bpy.types.Object, bpy.types.Object]:
    body = bpy.data.objects.get("Body")
    face = bpy.data.objects.get("Face")
    hair = bpy.data.objects.get("Hair001")
    if not body or not face or not hair:
        raise RuntimeError("The CC0 VRM must contain Body, Face, and Hair001 meshes")

    navy_skirt = toon_material("ArisCC0_Navy_Skirt", (0.025, 0.055, 0.16, 1.0))
    navy_boot = toon_material("ArisCC0_Navy_Boot", (0.02, 0.035, 0.10, 1.0), metallic=0.08)

    source_deep_hair = hair.data.materials[0]
    source_blue_hair = hair.data.materials[1] if len(hair.data.materials) > 1 else source_deep_hair
    if source_deep_hair is None or source_blue_hair is None:
        raise RuntimeError("The CC0 hair mesh is missing its source materials")
    deep_hair = clone_tinted_hair_material(source_deep_hair, "ArisCC0_Hair_Deep", 0.62, (0.025, 0.065, 0.22))
    blue_hair = clone_tinted_hair_material(source_blue_hair, "ArisCC0_Hair_Blue", 0.57, (0.025, 0.15, 0.34))
    if hair_mode == "custom":
        bpy.data.objects.remove(hair, do_unlink=True)
    else:
        repair_base_hair(hair, deep_hair, blue_hair)
    # The VRoid sample uses one clothing slot for top and skirt. Its skirt is
    # below z≈1.05, so only that region receives the navy Aris palette; the
    # white upper garment and original body texture remain intact.
    skirt_slot = len(body.data.materials)
    body.data.materials.append(navy_skirt)
    for polygon in body.data.polygons:
        center = body.matrix_world @ polygon.center
        if polygon.material_index == 1 and center.z < 1.05:
            polygon.material_index = skirt_slot
    replace_slot(body.data, 2, navy_boot)
    return body, face


def add_aris_details(armature: bpy.types.Object, hair_mode: str) -> None:
    skin = toon_material("ArisCC0_Skin", (0.98, 0.76, 0.74, 1.0))
    teal_hair = toon_material("ArisCC0_Hair_Teal", (0.02, 0.58, 0.70, 1.0), metallic=0.08)
    coat = toon_material("ArisCC0_Coat", (0.88, 0.93, 0.99, 1.0))
    coat_shadow = toon_material("ArisCC0_Coat_Shadow", (0.55, 0.66, 0.84, 1.0))
    navy = toon_material("ArisCC0_Navy", (0.025, 0.045, 0.14, 1.0))
    teal = toon_material("ArisCC0_Teal", (0.02, 0.64, 0.76, 1.0), metallic=0.15)
    gold = toon_material("ArisCC0_Gold", (0.95, 0.55, 0.12, 1.0), metallic=0.45)
    white = toon_material("ArisCC0_Halo", (0.88, 0.94, 1.0, 1.0), metallic=0.05)
    device = toon_material("ArisCC0_Device", (0.012, 0.022, 0.08, 1.0), metallic=0.15)

    if hair_mode == "custom":
        deep_hair = toon_material("ArisCC0_Hair_Custom_Deep", (0.018, 0.045, 0.16, 1.0), metallic=0.05)
        blue_hair = toon_material("ArisCC0_Hair_Custom_Blue", (0.025, 0.18, 0.40, 1.0), metallic=0.04)
        # Fully original fallback hair: rounded cap, layered bangs, curved
        # ponytails and separate teal tips.
        hair_cap = authored.add_uv_sphere("aris_cc0_hair_cap", (0.0, 0.10, 1.57), (0.215, 0.11, 0.18), deep_hair, segments=48, rings=32)
        parent_to_bone(hair_cap, armature, "J_Bip_C_Head")
        bang_specs = [
            ("left", [(-0.17, 1.64), (-0.04, 1.67), (-0.05, 1.57), (-0.13, 1.58)], blue_hair),
            ("center", [(-0.05, 1.67), (0.05, 1.67), (0.03, 1.56), (-0.01, 1.55)], deep_hair),
            ("right", [(0.04, 1.67), (0.17, 1.64), (0.13, 1.58), (0.05, 1.57)], blue_hair),
        ]
        for label, points, surface in bang_specs:
            bang = add_extruded_xz(f"aris_cc0_{label}_bang", points, -0.145, 0.045, surface, bevel=0.014)
            parent_to_bone(bang, armature, "J_Bip_C_Head")
        for sign, label in ((-1.0, "left"), (1.0, "right")):
            side_lock = authored.add_curve_lock(
                f"aris_cc0_{label}_side_lock",
                [(0.20 * sign, -0.005, 1.58), (0.30 * sign, -0.005, 1.42), (0.34 * sign, 0.01, 1.12), (0.29 * sign, 0.02, 0.88)],
                0.030,
                deep_hair if sign < 0 else blue_hair,
            )
            parent_to_bone(side_lock, armature, "J_Bip_C_Head")
            ponytail = authored.add_curve_lock(
                f"aris_cc0_{label}_ponytail",
                [(0.22 * sign, 0.07, 1.56), (0.33 * sign, 0.10, 1.49), (0.43 * sign, 0.09, 1.28), (0.39 * sign, 0.06, 1.02)],
                0.055,
                deep_hair if sign < 0 else blue_hair,
            )
            parent_to_bone(ponytail, armature, "J_Bip_C_Head")
            ponytail_highlight = authored.add_curve_lock(
                f"aris_cc0_{label}_ponytail_highlight",
                [(0.27 * sign, -0.015, 1.48), (0.36 * sign, -0.01, 1.33), (0.39 * sign, 0.00, 1.13)],
                0.018,
                blue_hair if sign < 0 else teal_hair,
            )
            parent_to_bone(ponytail_highlight, armature, "J_Bip_C_Head")
            tip = authored.add_curve_lock(
                f"aris_cc0_{label}_ponytail_tip",
                [(0.39 * sign, 0.06, 1.08), (0.38 * sign, 0.05, 0.99), (0.34 * sign, 0.04, 0.93)],
                0.035,
                teal_hair,
            )
            parent_to_bone(tip, armature, "J_Bip_C_Head")

    # Open jacket lapels: the CC0 base carries the facial/limb topology and
    # the additions only establish Aris's white/navy/teal garment language.
    for sign, label in ((-1.0, "left"), (1.0, "right")):
        panel = authored.add_rounded_box(
            f"aris_cc0_{label}_coat_panel",
            (0.18 * sign, -0.278, 1.18),
            (0.065, 0.035, 0.26),
            coat,
            bevel=0.022,
        )
        parent_to_bone(panel, armature, "J_Bip_C_Chest")
        trim = authored.add_rounded_box(
            f"aris_cc0_{label}_coat_trim",
            (0.145 * sign, -0.304, 1.18),
            (0.012, 0.018, 0.24),
            teal,
            bevel=0.008,
        )
        parent_to_bone(trim, armature, "J_Bip_C_Chest")

    collar_left = authored.add_rounded_box("aris_cc0_collar_left", (-0.095, -0.31, 1.385), (0.16, 0.025, 0.035), coat_shadow, bevel=0.012)
    collar_right = authored.add_rounded_box("aris_cc0_collar_right", (0.095, -0.31, 1.385), (0.16, 0.025, 0.035), coat_shadow, bevel=0.012)
    for collar, sign in ((collar_left, -1.0), (collar_right, 1.0)):
        collar.rotation_euler[1] = math.radians(8.0) * sign
        parent_to_bone(collar, armature, "J_Bip_C_Chest")

    tie = authored.add_rounded_box("aris_cc0_teal_tie", (0.0, -0.325, 1.255), (0.065, 0.025, 0.22), teal, bevel=0.018)
    parent_to_bone(tie, armature, "J_Bip_C_Chest")
    tie_clip = authored.add_uv_sphere("aris_cc0_tie_clip", (0.0, -0.345, 1.35), (0.033, 0.018, 0.033), gold, segments=20, rings=12)
    parent_to_bone(tie_clip, armature, "J_Bip_C_Chest")
    for sign, label in ((-1.0, "left"), (1.0, "right")):
        for index, z in enumerate((1.13, 1.26)):
            button = authored.add_uv_sphere(
                f"aris_cc0_{label}_button_{index}",
                (0.245 * sign, -0.325, z),
                (0.014, 0.012, 0.014),
                gold,
                segments=16,
                rings=10,
            )
            parent_to_bone(button, armature, "J_Bip_C_Chest")

    # Navy skirt accents remain separate from the CC0 body texture so the
    # original mesh can still be swapped without changing the accessories.
    waistband = authored.add_torus("aris_cc0_skirt_waistband", (0.0, 0.0, 1.01), 0.29, 0.016, teal)
    parent_to_bone(waistband, armature, "J_Bip_C_Hips")
    for index, x in enumerate((-0.24, -0.16, -0.08, 0.0, 0.08, 0.16, 0.24)):
        pleat = authored.add_rounded_box(
            f"aris_cc0_skirt_pleat_{index}",
            (x, -0.24, 0.82),
            (0.018, 0.018, 0.30),
            navy if index % 2 else coat_shadow,
            bevel=0.006,
        )
        parent_to_bone(pleat, armature, "J_Bip_C_Hips")

    # Vertical halo in the X/Z plane, positioned behind the head.
    halo = authored.add_torus(
        "aris_cc0_halo_outer",
        (0.0, 0.13, 1.56),
        0.43,
        0.022,
        white,
        rotation=(math.pi / 2, 0.0, 0.0),
    )
    parent_to_bone(halo, armature, "J_Bip_C_Head")
    halo_inner = authored.add_torus(
        "aris_cc0_halo_inner",
        (0.0, 0.145, 1.56),
        0.32,
        0.008,
        teal,
        rotation=(math.pi / 2, 0.0, 0.0),
    )
    parent_to_bone(halo_inner, armature, "J_Bip_C_Head")
    for index, (x, z, angle) in enumerate(((-0.36, 1.68, -0.35), (0.36, 1.68, 0.35), (0.0, 1.98, 0.0))):
        marker = authored.add_rounded_box(
            f"aris_cc0_halo_marker_{index}",
            (x, 0.11, z),
            (0.05, 0.028, 0.12),
            teal,
            bevel=0.012,
        )
        marker.rotation_euler[1] = angle
        parent_to_bone(marker, armature, "J_Bip_C_Head")

    # Handheld game device as a separate, easily removable accessory.
    console = authored.add_rounded_box("aris_cc0_handheld_console", (-0.54, -0.26, 1.16), (0.20, 0.065, 0.12), device, bevel=0.025)
    console.rotation_euler[1] = math.radians(-12.0)
    parent_to_bone(console, armature, "J_Bip_L_Hand")
    screen = authored.add_rounded_box("aris_cc0_handheld_screen", (-0.54, -0.30, 1.17), (0.12, 0.012, 0.075), teal, bevel=0.008)
    screen.rotation_euler[1] = math.radians(-12.0)
    parent_to_bone(screen, armature, "J_Bip_L_Hand")
    for index, x in enumerate((-0.595, -0.49)):
        button = authored.add_uv_sphere(f"aris_cc0_console_button_{index}", (x, -0.31, 1.11), (0.012, 0.008, 0.012), gold, segments=12, rings=8)
        parent_to_bone(button, armature, "J_Bip_L_Hand")


def setup_render(render_path: Path) -> None:
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_EEVEE_NEXT"
    scene.render.resolution_x = 640
    scene.render.resolution_y = 800
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "PNG"
    scene.render.filepath = str(render_path)
    scene.render.film_transparent = False
    scene.world.color = (0.012, 0.020, 0.055)

    bpy.ops.object.camera_add(location=(0.0, -4.5, 1.02))
    camera = bpy.context.object
    assert camera is not None
    camera.data.lens = 62
    camera.rotation_euler = (Vector((0.0, 0.0, 0.88)) - camera.location).to_track_quat("-Z", "Y").to_euler()
    scene.camera = camera

    def area_light(name: str, location: Point, energy: float, size: float, color: Color) -> None:
        bpy.ops.object.light_add(type="AREA", location=location)
        light = bpy.context.object
        assert light is not None
        light.name = name
        light.data.energy = energy
        light.data.shape = "DISK"
        light.data.size = size
        light.data.color = color[:3]
        light.rotation_euler = (Vector((0.0, 0.0, 0.9)) - light.location).to_track_quat("-Z", "Y").to_euler()

    area_light("ArisCC0_KeyLight", (3.0, -3.5, 3.2), 720.0, 3.0, (0.82, 0.92, 1.0, 1.0))
    area_light("ArisCC0_FillLight", (-3.0, -2.0, 2.0), 420.0, 2.5, (0.34, 0.72, 1.0, 1.0))
    area_light("ArisCC0_RimLight", (0.0, 1.8, 2.8), 850.0, 2.0, (0.20, 0.95, 1.0, 1.0))


def main() -> None:
    args = parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.blend.parent.mkdir(parents=True, exist_ok=True)
    args.render.parent.mkdir(parents=True, exist_ok=True)
    if not args.base.exists():
        raise FileNotFoundError(args.base)

    clear_scene()
    bpy.ops.import_scene.vrm(filepath=str(args.base))
    armature = next((obj for obj in bpy.context.scene.objects if obj.type == "ARMATURE"), None)
    if armature is None:
        raise RuntimeError("Base VRM did not import an armature")

    recolor_base(armature, args.hair_mode)
    add_aris_details(armature, args.hair_mode)
    setup_render(args.render)

    extension = get_armature_extension(armature.data)
    if hasattr(extension, "vrm0"):
        extension.vrm0.meta.title = "Aris-inspired CC0 base study"
        extension.vrm0.meta.version = "0.1.0"
        extension.vrm0.meta.author = "desktop-ai-companion contributors"
        extension.vrm0.meta.contact_information = "desktop-ai-companion local prototype"
        extension.vrm0.meta.reference = "CC0 VRoid sample base; original accessories authored in this repository"

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
    bpy.ops.render.render(write_still=True)
    bpy.ops.wm.save_as_mainfile(filepath=str(args.blend))
    print(f"Generated VRM: {args.out}")
    print(f"Generated Blend: {args.blend}")
    print(f"Generated Render: {args.render}")


if __name__ == "__main__":
    main()
