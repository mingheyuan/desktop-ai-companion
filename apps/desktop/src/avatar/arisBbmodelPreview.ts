import * as THREE from "three";
import gunData from "./assets/aris-13a-gun.json";
import modelData from "./assets/aris-13a-main.json";
import type { ArisBlockPreview } from "./arisBlockPreview";

type BbTexture = {
  name: string;
  source?: string;
  width?: number;
  height?: number;
  uv_width?: number;
  uv_height?: number;
};

type BbFace = {
  uv?: number[];
  texture?: number | null;
  rotation?: number;
};

type BbElement = {
  uuid: string;
  name: string;
  from: number[];
  to: number[];
  origin?: number[];
  rotation?: number[] | null;
  inflate?: number | null;
  export?: boolean;
  faces: Record<string, BbFace>;
};

type BbGroup = {
  uuid: string;
  name: string;
  origin?: number[];
  rotation?: number[] | null;
};

type BbOutlinerEntry = string | { uuid: string; children?: BbOutlinerEntry[] };

type BbModel = {
  resolution: { width: number; height: number };
  textures: BbTexture[];
  elements: BbElement[];
  groups: BbGroup[];
  outliner: BbOutlinerEntry[];
};

const PIXELS_PER_UNIT = 16;
const sourceModel = modelData as unknown as BbModel;
const sourceGunModel = gunData as unknown as BbModel;
// bbVector mirrors Blockbench's Z axis so the model faces the Three.js camera
// at +Z; consequently the front `north` face occupies BoxGeometry's +Z slot.
const faceOrder = ["east", "west", "up", "down", "north", "south"] as const;

function bbVector(values: number[]) {
  return new THREE.Vector3(
    values[0] / PIXELS_PER_UNIT,
    values[1] / PIXELS_PER_UNIT,
    -(values[2] ?? 0) / PIXELS_PER_UNIT,
  );
}

function bbOffset(values: number[], parentOrigin: THREE.Vector3) {
  return bbVector(values).sub(parentOrigin);
}

function materialForTexture(texture: THREE.Texture) {
  return new THREE.MeshStandardMaterial({
    map: texture,
    roughness: 0.78,
    metalness: 0.02,
    transparent: true,
    alphaTest: 0.04,
    side: THREE.DoubleSide,
  });
}

function invisibleMaterial() {
  return new THREE.MeshBasicMaterial({
    transparent: true,
    opacity: 0,
    depthWrite: false,
  });
}

function setFaceUvs(
  geometry: THREE.BoxGeometry,
  faceIndex: number,
  face: BbFace | undefined,
  resolution: { width: number; height: number },
) {
  const uv = face?.uv;
  if (!uv || uv.length < 4) return;
  const [u1, v1, u2, v2] = uv;
  const corners = [
    [u1 / resolution.width, 1 - v1 / resolution.height],
    [u2 / resolution.width, 1 - v1 / resolution.height],
    [u1 / resolution.width, 1 - v2 / resolution.height],
    [u2 / resolution.width, 1 - v2 / resolution.height],
  ];
  let rotation = ((face.rotation ?? 0) % 360 + 360) % 360;
  while (rotation >= 90) {
    const [first, second, third, fourth] = corners;
    corners[0] = third;
    corners[1] = first;
    corners[2] = fourth;
    corners[3] = second;
    rotation -= 90;
  }
  const attribute = geometry.getAttribute("uv");
  for (let index = 0; index < 4; index += 1) {
    attribute.setXY(faceIndex * 4 + index, corners[index][0], corners[index][1]);
  }
  attribute.needsUpdate = true;
}

function createCube(
  element: BbElement,
  model: BbModel,
  textures: THREE.Material[],
  hiddenSurface: THREE.Material,
  parentOrigin: THREE.Vector3,
) {
  const inflate = element.inflate ?? 0;
  const from = element.from;
  const to = element.to;
  // Blockbench uses zero-thickness cubes for eyes, mouth, patches, and the
  // halo ring. Give those planes a tiny volume so Three.js still rasterizes
  // their textured face instead of dropping a degenerate box.
  const width = Math.max((to[0] - from[0] + inflate * 2) / PIXELS_PER_UNIT, 0.002);
  const height = Math.max((to[1] - from[1] + inflate * 2) / PIXELS_PER_UNIT, 0.002);
  const depth = Math.max((to[2] - from[2] + inflate * 2) / PIXELS_PER_UNIT, 0.002);
  const geometry = new THREE.BoxGeometry(width, height, depth);
  const materials = faceOrder.map((faceName, index) => {
    const face = element.faces[faceName];
    const textureIndex = face?.texture;
    const texture = textureIndex == null ? undefined : model.textures[textureIndex];
    setFaceUvs(geometry, index, face, {
      width: texture?.uv_width ?? texture?.width ?? model.resolution.width,
      height: texture?.uv_height ?? texture?.height ?? model.resolution.height,
    });
    return textureIndex == null ? hiddenSurface : textures[textureIndex] ?? hiddenSurface;
  });
  const part = new THREE.Mesh(geometry, materials);
  const center = [
    (from[0] + to[0]) / 2,
    (from[1] + to[1]) / 2,
    (from[2] + to[2]) / 2,
  ];
  part.position.copy(bbOffset(center, parentOrigin));
  // Face overlays are intentionally only 0.01 pixels in front of the head in
  // the source model. Move north-facing zero-depth planes forward so the
  // depth buffer does not hide eyes and mouth behind the head cube.
  if (Math.abs(to[2] - from[2]) < 0.0001 && element.faces.north?.texture != null) {
    part.position.z += 0.05;
  }
  if (element.rotation) {
    part.rotation.set(
      -THREE.MathUtils.degToRad(element.rotation[0] ?? 0),
      THREE.MathUtils.degToRad(element.rotation[1] ?? 0),
      -THREE.MathUtils.degToRad(element.rotation[2] ?? 0),
    );
  }
  part.name = `aris-bbmodel-${element.name}`;
  part.frustumCulled = false;
  return part;
}

function shouldSkipElement(element: BbElement) {
  return element.export === false || /^Camera\d/.test(element.name);
}

function shouldSkipGroup(group: BbGroup) {
  return group.name === "CameraAnchor" || group.name === "NameplateAnchor";
}

/**
 * Loads the user-authorized Figura/Blockbench Aris study as a Three.js preview.
 * The source model is bundled as JSON because BBModel embeds its small pixel
 * textures as data URLs. It is intentionally kept behind an explicit preview
 * switch and does not replace the normal VRM renderer.
 */
function buildBbmodelPreview(model: BbModel, rootName: string) {
  const group = new THREE.Group();
  group.name = rootName;

  const textureLoader = new THREE.TextureLoader();
  const textureMaterials: THREE.Material[] = [];
  let haloMaterial: THREE.MeshStandardMaterial | undefined;
  for (const texture of model.textures) {
    const loaded = texture.source ? textureLoader.load(texture.source) : new THREE.Texture();
    loaded.colorSpace = THREE.SRGBColorSpace;
    // BBModel UVs use pixel coordinates from the top-left of the image. Keep
    // the image upload unflipped and convert the V coordinate explicitly in
    // setFaceUvs, matching the reference three-blockbench loader.
    loaded.flipY = false;
    loaded.magFilter = THREE.NearestFilter;
    loaded.minFilter = THREE.NearestFilter;
    loaded.wrapS = THREE.ClampToEdgeWrapping;
    loaded.wrapT = THREE.ClampToEdgeWrapping;
    const surface = materialForTexture(loaded);
    textureMaterials.push(surface);
    if (texture.name === "halo.png") haloMaterial = surface;
  }
  const hiddenSurface = invisibleMaterial();

  const elements = new Map(model.elements.map((element) => [element.uuid, element]));
  const groups = new Map(model.groups.map((entry) => [entry.uuid, entry]));
  const elementObjects = new Map<string, THREE.Object3D>();
  const groupObjects = new Map<string, THREE.Object3D>();
  const placedElements = new Set<string>();

  const addEntry = (entry: BbOutlinerEntry, parent: THREE.Object3D, parentOrigin: THREE.Vector3) => {
    if (typeof entry === "string") {
      const element = elements.get(entry);
      if (!element || shouldSkipElement(element)) return;
      const part = createCube(element, model, textureMaterials, hiddenSurface, parentOrigin);
      parent.add(part);
      elementObjects.set(element.name, part);
      placedElements.add(element.uuid);
      return;
    }
    const definition = groups.get(entry.uuid);
    if (!definition || shouldSkipGroup(definition)) return;
    const node = new THREE.Group();
    node.name = `aris-bbmodel-group-${definition.name}`;
    const origin = bbVector(definition.origin ?? [0, 0, 0]);
    node.position.copy(origin.clone().sub(parentOrigin));
    const rotation = definition.rotation;
    if (rotation) {
      node.rotation.set(
        -THREE.MathUtils.degToRad(rotation[0] ?? 0),
        THREE.MathUtils.degToRad(rotation[1] ?? 0),
        -THREE.MathUtils.degToRad(rotation[2] ?? 0),
      );
    }
    parent.add(node);
    groupObjects.set(definition.name, node);
    for (const child of entry.children ?? []) addEntry(child, node, origin);
  };

  for (const entry of model.outliner) addEntry(entry, group, new THREE.Vector3());
  for (const element of model.elements) {
    if (placedElements.has(element.uuid) || shouldSkipElement(element)) continue;
    group.add(createCube(element, model, textureMaterials, hiddenSurface, new THREE.Vector3()));
  }

  group.scale.setScalar(0.68);
  const head = groupObjects.get("Head") ?? group;
  const body = groupObjects.get("UpperBody") ?? groupObjects.get("Body") ?? group;
  const halo = groupObjects.get("Halo") ?? groupObjects.get("HaloCenter") ?? group;
  const leftEye = elementObjects.get("LeftEye") ?? group;
  const rightEye = elementObjects.get("RightEye") ?? group;
  const mouth = elementObjects.get("Mouth") ?? group;
  const accentMaterial = new THREE.MeshStandardMaterial({
    color: 0x30b9ca,
    roughness: 0.34,
    metalness: 0.18,
    transparent: true,
    opacity: 0,
  });

  return {
    group,
    head,
    body,
    leftEye,
    rightEye,
    mouth,
    halo,
    accentMaterial,
    haloMaterial: haloMaterial ?? materialForTexture(new THREE.Texture()),
  };
}

/**
 * Builds the normal outfit and the author's separate rail-gun model. The
 * weapon uses the reference package's stored-on-back presentation by default;
 * the compact preview can hide it with `?avatar=block&weapon=off`.
 */
export function createArisBbmodelPreview(): ArisBlockPreview {
  const main = buildBbmodelPreview(sourceModel, "aris-bbmodel-preview");
  const weapon = buildBbmodelPreview(sourceGunModel, "aris-railgun-preview");
  const group = new THREE.Group();
  group.name = "aris-bbmodel-avatar";
  group.position.y = 0.06;
  group.add(main.group, weapon.group);

  weapon.group.scale.setScalar(1.496);
  weapon.group.position.copy(bbVector([0, 13.5, 6]));
  weapon.group.rotation.set(0, THREE.MathUtils.degToRad(-90), THREE.MathUtils.degToRad(32.5));
  weapon.group.visible = true;

  return { ...main, group, weapon: weapon.group };
}
