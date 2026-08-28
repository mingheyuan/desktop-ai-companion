import * as THREE from "three";
import { RoundedBoxGeometry } from "three/examples/jsm/geometries/RoundedBoxGeometry.js";

export interface ArisBlockPreview {
  group: THREE.Group;
  head: THREE.Object3D;
  body: THREE.Object3D;
  leftEye: THREE.Object3D;
  rightEye: THREE.Object3D;
  mouth: THREE.Object3D;
  halo: THREE.Object3D;
  accentMaterial: THREE.MeshStandardMaterial;
  haloMaterial: THREE.MeshStandardMaterial;
}

const vector = (x: number, y: number, z: number) => new THREE.Vector3(x, y, z);

function material(color: number, options: Partial<THREE.MeshStandardMaterialParameters> = {}) {
  return new THREE.MeshStandardMaterial({
    color,
    roughness: 0.62,
    metalness: 0.02,
    flatShading: true,
    ...options,
  });
}

function mesh<T extends THREE.BufferGeometry>(geometry: T, surface: THREE.Material) {
  const part = new THREE.Mesh(geometry, surface);
  part.castShadow = false;
  part.receiveShadow = false;
  return part;
}

function block(
  parent: THREE.Object3D,
  position: THREE.Vector3,
  dimensions: THREE.Vector3,
  surface: THREE.Material,
  bevel = 0,
) {
  const geometry = bevel > 0
    ? new RoundedBoxGeometry(dimensions.x, dimensions.y, dimensions.z, 2, bevel)
    : new THREE.BoxGeometry(dimensions.x, dimensions.y, dimensions.z);
  const part = mesh(geometry, surface);
  part.position.copy(position);
  parent.add(part);
  return part;
}

function blockBetween(
  parent: THREE.Object3D,
  start: THREE.Vector3,
  end: THREE.Vector3,
  width: number,
  depth: number,
  surface: THREE.Material,
) {
  const direction = end.clone().sub(start);
  const part = block(parent, start.clone().add(end).multiplyScalar(0.5), vector(width, direction.length(), depth), surface, 0.015);
  part.quaternion.setFromUnitVectors(new THREE.Vector3(0, 1, 0), direction.normalize());
  return part;
}

function eye(parent: THREE.Object3D, x: number, surface: THREE.MeshStandardMaterial) {
  const eyePart = block(parent, vector(x, -0.02, 0.235), vector(0.12, 0.17, 0.035), surface, 0.018);
  const pupil = block(parent, vector(x, -0.02, 0.258), vector(0.05, 0.095, 0.018), material(0x15385f, {
    roughness: 0.24,
    metalness: 0.14,
    emissive: 0x08243e,
    emissiveIntensity: 0.28,
  }), 0.008);
  block(parent, vector(x - Math.sign(x) * 0.018, 0.023, 0.27), vector(0.018, 0.026, 0.012), material(0xffffff, { flatShading: false }), 0.005);
  pupil.name = `aris-block-${x < 0 ? "left" : "right"}-pupil`;
  return eyePart;
}

/**
 * An original block-built Aris study. It uses the Minecraft/Blockbench visual
 * language, but is rendered directly in Three.js so it can be previewed before
 * exporting a standalone GLB. It intentionally has no third-party textures.
 */
export function createArisBlockPreview(): ArisBlockPreview {
  const group = new THREE.Group();
  group.name = "aris-block-preview";

  const skin = material(0xffd9d1, { roughness: 0.76 });
  const hairSurface = material(0x2a3556, { roughness: 0.42, metalness: 0.04 });
  const hairShadow = material(0x1a2440, { roughness: 0.46 });
  const hairTip = material(0x2db9c8, {
    roughness: 0.32,
    metalness: 0.1,
    emissive: 0x063941,
    emissiveIntensity: 0.16,
  });
  const coat = material(0xf1f5fb, { roughness: 0.68 });
  const coatShadow = material(0xc4d2e9, { roughness: 0.72 });
  const navy = material(0x162044, { roughness: 0.5 });
  const dark = material(0x0b122a, { roughness: 0.54 });
  const accentMaterial = material(0x30b9ca, {
    roughness: 0.34,
    metalness: 0.18,
    emissive: 0x063b47,
    emissiveIntensity: 0.22,
  });
  const haloMaterial = material(0xbcefff, {
    roughness: 0.22,
    metalness: 0.32,
    emissive: 0x2b9bb3,
    emissiveIntensity: 0.72,
    transparent: true,
    opacity: 0.94,
  });
  const gold = material(0xffd36f, {
    roughness: 0.3,
    metalness: 0.48,
    emissive: 0x50320a,
    emissiveIntensity: 0.2,
  });
  const mouthMaterial = new THREE.MeshBasicMaterial({ color: 0x552e48 });

  // Layered back hair follows the reference's stepped Blockbench silhouette.
  // Keeping it on a separate group makes the body read clearly while preserving
  // the long-hair shape at desktop-pet scale.
  const backHair = new THREE.Group();
  backHair.name = "aris-block-back-hair";
  for (const [y, width, height, surface] of [
    [1.54, 0.48, 0.42, hairShadow],
    [1.22, 0.62, 0.32, hairSurface],
    [0.94, 0.72, 0.24, hairSurface],
    [0.72, 0.78, 0.18, hairShadow],
    [0.54, 0.7, 0.16, hairSurface],
  ] as const) {
    for (const sign of [-1, 1] as const) {
      const lock = block(backHair, vector(sign * width, y, -0.18), vector(0.24, height, 0.28), surface, 0.02);
      lock.rotation.z = -sign * 0.025;
    }
  }
  for (const sign of [-1, 1] as const) {
    block(backHair, vector(sign * 0.56, 0.39, -0.18), vector(0.23, 0.22, 0.27), hairSurface, 0.02);
    block(backHair, vector(sign * 0.58, 0.51, -0.03), vector(0.08, 0.06, 0.32), accentMaterial, 0.01);
  }
  group.add(backHair);

  const body = new THREE.Group();
  body.name = "aris-block-body";
  // Legs, socks and hard-edged sneakers.
  blockBetween(body, vector(-0.17, 0.24, 0), vector(-0.19, 0.72, 0), 0.16, 0.16, skin);
  blockBetween(body, vector(0.17, 0.24, 0), vector(0.19, 0.72, 0), 0.16, 0.16, skin);
  for (const [x, tilt] of [[-0.19, -0.06], [0.19, 0.06]] as const) {
    const sock = block(body, vector(x, 0.25, 0.02), vector(0.17, 0.27, 0.18), navy, 0.018);
    sock.rotation.z = tilt;
    const shoe = block(body, vector(x, 0.09, 0.12), vector(0.28, 0.18, 0.42), coat, 0.025);
    shoe.rotation.y = tilt;
    const sole = block(body, vector(x, 0.012, 0.12), vector(0.29, 0.045, 0.44), dark, 0.008);
    sole.rotation.y = tilt;
    const tongue = block(body, vector(x, 0.19, 0.185), vector(0.09, 0.04, 0.19), accentMaterial, 0.008);
    tongue.rotation.y = tilt;
  }

  // Navy pleated skirt: a low-poly octagonal volume plus individual front pleats.
  const skirt = mesh(new THREE.CylinderGeometry(0.39, 0.54, 0.4, 8), navy);
  skirt.position.set(0, 0.99, 0);
  skirt.rotation.y = Math.PI / 8;
  body.add(skirt);
  for (let index = -3; index <= 3; index += 1) {
    const pleat = block(body, vector(index * 0.095, 0.91, 0.47), vector(0.045, 0.32, 0.028), index % 2 === 0 ? coatShadow : accentMaterial, 0.006);
    pleat.rotation.z = index * 0.025;
  }
  block(body, vector(0, 1.19, 0), vector(0.72, 0.06, 0.44), accentMaterial, 0.018);

  // Uniform body and open jacket.
  block(body, vector(0, 1.47, 0), vector(0.58, 0.47, 0.34), dark, 0.045);
  block(body, vector(0, 1.52, -0.035), vector(0.79, 0.5, 0.25), coat, 0.035);
  for (const [x, tilt] of [[-0.28, -0.09], [0.28, 0.09]] as const) {
    const lapel = block(body, vector(x, 1.49, 0.19), vector(0.18, 0.42, 0.045), coatShadow, 0.012);
    lapel.rotation.z = tilt;
    const trim = block(body, vector(x * 0.78, 1.49, 0.222), vector(0.035, 0.38, 0.018), accentMaterial, 0.006);
    trim.rotation.z = tilt;
    const pocket = block(body, vector(x * 0.86, 1.26, 0.225), vector(0.16, 0.10, 0.028), dark, 0.012);
    pocket.rotation.z = tilt;
    block(body, vector(x * 0.88, 1.73, 0.17), vector(0.2, 0.034, 0.026), accentMaterial, 0.006).rotation.z = tilt;
  }
  // Small uniform patches echo the Millennium/GDD identifiers in the reference.
  block(body, vector(-0.34, 1.58, 0.205), vector(0.055, 0.09, 0.022), accentMaterial, 0.006).rotation.z = -0.06;
  block(body, vector(-0.34, 1.47, 0.212), vector(0.095, 0.024, 0.02), gold, 0.004).rotation.z = -0.06;
  block(body, vector(0, 1.76, 0.22), vector(0.3, 0.1, 0.08), coat, 0.02);
  const tie = block(body, vector(0, 1.57, 0.245), vector(0.10, 0.3, 0.035), accentMaterial, 0.01);
  tie.rotation.z = Math.PI;
  block(body, vector(0, 1.7, 0.27), vector(0.07, 0.05, 0.028), gold, 0.008);
  group.add(body);

  // Box-built sleeves and hands.
  const arms = new THREE.Group();
  blockBetween(arms, vector(-0.34, 1.66, 0), vector(-0.61, 1.29, 0.02), 0.22, 0.25, coat);
  blockBetween(arms, vector(0.34, 1.66, 0), vector(0.61, 1.29, 0.02), 0.22, 0.25, coat);
  block(arms, vector(-0.63, 1.22, 0.04), vector(0.18, 0.18, 0.18), skin, 0.025);
  block(arms, vector(0.63, 1.22, 0.04), vector(0.18, 0.18, 0.18), skin, 0.025);
  block(arms, vector(-0.58, 1.31, 0.04), vector(0.25, 0.12, 0.28), navy, 0.015).rotation.z = -0.35;
  block(arms, vector(0.58, 1.31, 0.04), vector(0.25, 0.12, 0.28), navy, 0.015).rotation.z = 0.35;
  const handheld = block(arms, vector(-0.5, 1.2, 0.22), vector(0.26, 0.18, 0.06), dark, 0.018);
  handheld.rotation.z = -0.18;
  const screen = block(arms, vector(-0.5, 1.2, 0.255), vector(0.16, 0.1, 0.012), accentMaterial, 0.006);
  screen.rotation.z = -0.18;
  group.add(arms);

  // Head pivot is centered at the face so look/idle rotations do not orbit the body.
  const head = new THREE.Group();
  head.name = "aris-block-head";
  head.position.set(0, 2.1, 0);
  block(head, vector(0, 0, 0.02), vector(0.62, 0.68, 0.42), skin, 0.065);
  block(head, vector(0, -0.37, 0), vector(0.16, 0.18, 0.16), skin, 0.025);
  const leftEye = eye(head, -0.14, material(0x54d0ff, { roughness: 0.18, metalness: 0.12, emissive: 0x0b6179, emissiveIntensity: 0.34 }));
  const rightEye = eye(head, 0.14, material(0x54d0ff, { roughness: 0.18, metalness: 0.12, emissive: 0x0b6179, emissiveIntensity: 0.34 }));
  block(head, vector(-0.14, 0.14, 0.236), vector(0.15, 0.025, 0.025), hairShadow, 0.006).rotation.z = -0.08;
  block(head, vector(0.14, 0.14, 0.236), vector(0.15, 0.025, 0.025), hairShadow, 0.006).rotation.z = 0.08;
  const mouth = block(head, vector(0, -0.16, 0.238), vector(0.095, 0.035, 0.018), mouthMaterial, 0.008);
  mouth.name = "aris-block-mouth";
  block(head, vector(0, -0.05, 0.245), vector(0.035, 0.028, 0.018), skin, 0.004);

  const hair = new THREE.Group();
  hair.name = "aris-block-hair";
  block(hair, vector(0, 0.27, -0.11), vector(0.7, 0.3, 0.46), hairShadow, 0.035);
  const bangSpecs = [
    [-0.23, 0.06, -0.11], [-0.08, 0.0, -0.03], [0.08, 0.0, -0.02], [0.23, 0.06, -0.1],
  ] as const;
  for (const [x, rotation, z] of bangSpecs) {
    const bang = block(hair, vector(x, 0.08, 0.235), vector(0.15, 0.29, 0.08), hairSurface, 0.018);
    bang.rotation.z = rotation;
    bang.position.z += z;
  }
  for (const sign of [-1, 1] as const) {
    const frontLock = block(hair, vector(sign * 0.3, -0.1, 0.255), vector(0.12, 0.43, 0.08), hairSurface, 0.016);
    frontLock.rotation.z = -sign * 0.08;
  }
  for (const sign of [-1, 1] as const) {
    const x = sign;
    const segments = [
      [0.34, 0.07, 0.28, hairShadow], [0.4, -0.25, 0.31, hairSurface], [0.45, -0.58, 0.31, hairSurface],
      [0.43, -0.91, 0.3, hairShadow], [0.36, -1.2, 0.23, hairTip],
    ] as const;
    for (const [offsetX, y, height, surface] of segments) {
      const lock = block(hair, vector(offsetX * x, y, -0.1), vector(0.19, height, 0.34), surface, 0.025);
      lock.rotation.z = 0.045 * x;
    }
    block(hair, vector(0.34 * x, 0.16, 0.02), vector(0.08, 0.07, 0.38), accentMaterial, 0.012).rotation.z = 0.12 * x;
  }
  head.add(hair);
  group.add(head);

  // Segmented circuit halo: deliberately open at the bottom to avoid covering the face.
  const halo = new THREE.Group();
  halo.name = "aris-block-halo";
  halo.position.set(0, 2.28, -0.08);
  for (const [x, y, width, height, rotation] of [
    [0, 0.52, 0.68, 0.055, 0], [-0.43, 0.35, 0.055, 0.38, -0.18], [0.43, 0.35, 0.055, 0.38, 0.18],
    [-0.3, 0.11, 0.18, 0.045, 0], [0.3, 0.11, 0.18, 0.045, 0],
  ] as const) {
    const segment = block(halo, vector(x, y, 0), vector(width, height, 0.035), haloMaterial, 0.012);
    segment.rotation.z = rotation;
  }
  block(halo, vector(0, 0.48, 0.022), vector(0.12, 0.045, 0.04), gold, 0.01);
  for (const [x, y] of [[-0.53, 0.38], [0.53, 0.38], [-0.42, 0.03], [0.42, 0.03]]) {
    block(halo, vector(x, y, 0.02), vector(0.05, 0.075, 0.04), accentMaterial, 0.01);
  }
  group.add(halo);

  // A compact block-built Supernova silhouette, kept behind the body.
  const weapon = new THREE.Group();
  weapon.name = "aris-block-supernova";
  weapon.position.set(0.57, 1.53, -0.3);
  weapon.rotation.z = -0.4;
  block(weapon, vector(0, 0, 0), vector(0.22, 1.0, 0.18), coatShadow, 0.025);
  block(weapon, vector(0, 0, 0.11), vector(0.09, 0.72, 0.06), accentMaterial, 0.012);
  block(weapon, vector(0, 0.46, 0.02), vector(0.3, 0.12, 0.22), coat, 0.02);
  block(weapon, vector(0, -0.54, 0), vector(0.14, 0.18, 0.16), dark, 0.018);
  block(weapon, vector(0.13, 0.16, 0.01), vector(0.08, 0.28, 0.22), dark, 0.014);
  block(weapon, vector(-0.08, 0.46, 0.14), vector(0.07, 0.06, 0.07), gold, 0.008);
  group.add(weapon);

  group.scale.setScalar(0.46);
  group.position.y = 0.2;
  return { group, head, body, leftEye, rightEye, mouth, halo, accentMaterial, haloMaterial };
}
