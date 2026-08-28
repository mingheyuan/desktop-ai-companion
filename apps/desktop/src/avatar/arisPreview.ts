import * as THREE from "three";
import { RoundedBoxGeometry } from "three/examples/jsm/geometries/RoundedBoxGeometry.js";

export interface ArisPreview {
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
    roughness: 0.58,
    metalness: 0.03,
    ...options,
  });
}

function mesh<T extends THREE.BufferGeometry>(geometry: T, surface: THREE.Material) {
  const part = new THREE.Mesh(geometry, surface);
  part.castShadow = false;
  part.receiveShadow = false;
  return part;
}

function roundedBox(width: number, height: number, depth: number, radius: number, surface: THREE.Material, segments = 3) {
  return mesh(new RoundedBoxGeometry(width, height, depth, segments, radius), surface);
}

function addCylinderBetween(
  parent: THREE.Object3D,
  start: THREE.Vector3,
  end: THREE.Vector3,
  radius: number,
  surface: THREE.Material,
  radialSegments = 12,
) {
  const direction = end.clone().sub(start);
  const length = direction.length();
  const part = mesh(new THREE.CapsuleGeometry(radius, Math.max(0.02, length - radius * 2), 6, radialSegments), surface);
  part.position.copy(start).add(end).multiplyScalar(0.5);
  part.quaternion.setFromUnitVectors(new THREE.Vector3(0, 1, 0), direction.normalize());
  parent.add(part);
  return part;
}

function addHairStrand(
  parent: THREE.Object3D,
  points: THREE.Vector3[],
  radius: number,
  surface: THREE.Material,
  tipSurface: THREE.Material,
) {
  const curve = new THREE.CatmullRomCurve3(points);
  const strand = mesh(new THREE.TubeGeometry(curve, 18, radius, 8, false), surface);
  parent.add(strand);

  const tipStart = points[Math.max(0, points.length - 2)].clone();
  const tipEnd = points[points.length - 1].clone();
  const tipDirection = tipEnd.clone().sub(tipStart);
  const tipCurve = new THREE.LineCurve3(tipStart, tipEnd);
  const tip = mesh(new THREE.TubeGeometry(tipCurve, 8, radius * 0.96, 8, false), tipSurface);
  tip.position.add(tipDirection.normalize().multiplyScalar(radius * 0.02));
  parent.add(tip);
}

function addHairRibbon(
  parent: THREE.Object3D,
  points: Array<[number, number]>,
  depth: number,
  surface: THREE.Material,
) {
  const shape = new THREE.Shape();
  shape.moveTo(points[0][0], points[0][1]);
  for (const [x, y] of points.slice(1)) shape.lineTo(x, y);
  shape.closePath();
  const geometry = new THREE.ExtrudeGeometry(shape, {
    depth,
    bevelEnabled: true,
    bevelSegments: 2,
    bevelSize: 0.012,
    bevelThickness: 0.012,
    curveSegments: 4,
  });
  const ribbon = mesh(geometry, surface);
  ribbon.position.z = 0.31;
  parent.add(ribbon);
  return ribbon;
}

function addEye(parent: THREE.Object3D, x: number, surface: THREE.Material) {
  const eye = mesh(new THREE.SphereGeometry(0.105, 20, 14), surface);
  eye.position.set(x, 2.105, 0.405);
  eye.scale.set(0.72, 1.32, 0.24);
  parent.add(eye);

  const pupil = mesh(new THREE.SphereGeometry(0.058, 16, 12), new THREE.MeshStandardMaterial({
    color: 0x164b77,
    roughness: 0.2,
    metalness: 0.1,
    emissive: 0x092b48,
    emissiveIntensity: 0.24,
  }));
  pupil.position.set(x, 2.095, 0.462);
  pupil.scale.set(0.66, 1.06, 0.12);
  parent.add(pupil);

  const highlight = mesh(new THREE.SphereGeometry(0.027, 12, 8), new THREE.MeshBasicMaterial({ color: 0xffffff }));
  highlight.position.set(x - Math.sign(x) * 0.025, 2.145, 0.468);
  highlight.scale.set(1, 1, 0.3);
  parent.add(highlight);
  return eye;
}

function addBrow(parent: THREE.Object3D, x: number, rotation: number, surface: THREE.Material) {
  const brow = roundedBox(0.16, 0.025, 0.018, 0.012, surface, 2);
  brow.position.set(x, 2.235, 0.412);
  brow.rotation.z = rotation;
  parent.add(brow);
}

/**
 * An original, asset-free preview assembled from simple meshes.
 * It is deliberately separate from the VRM path so an authorized model can
 * replace it without changing the desktop state or interaction contracts.
 */
export function createArisPreview(): ArisPreview {
  const group = new THREE.Group();
  group.name = "aris-original-preview";

  const skin = material(0xffded8, { roughness: 0.74 });
  const hair = material(0x202946, { roughness: 0.38, metalness: 0.06 });
  const hairShadow = material(0x151b35, { roughness: 0.42, metalness: 0.04 });
  const hairTip = material(0x35b5c7, { roughness: 0.3, metalness: 0.1, emissive: 0x06343d, emissiveIntensity: 0.12 });
  const coat = material(0xf0f4fb, { roughness: 0.66 });
  const coatShadow = material(0xd5deee, { roughness: 0.7 });
  const navy = material(0x151d38, { roughness: 0.46 });
  const dark = material(0x0b1024, { roughness: 0.5 });
  const accentMaterial = material(0x35b5c7, { roughness: 0.35, metalness: 0.16, emissive: 0x063c48, emissiveIntensity: 0.18 });
  const eyeMaterial = material(0x4fc9ff, { roughness: 0.18, metalness: 0.12, emissive: 0x0b5870, emissiveIntensity: 0.34 });
  const mouthMaterial = new THREE.MeshBasicMaterial({ color: 0x512d49 });
  const haloMaterial = material(0x91e8ff, { roughness: 0.22, metalness: 0.28, emissive: 0x2594b2, emissiveIntensity: 0.72, transparent: true, opacity: 0.92 });
  const gold = material(0xffd878, { roughness: 0.3, metalness: 0.45, emissive: 0x5b3c08, emissiveIntensity: 0.22 });
  const white = new THREE.MeshBasicMaterial({ color: 0xffffff });

  // Legs, boots and the pleated skirt silhouette.
  const legs = new THREE.Group();
  addCylinderBetween(legs, vector(-0.19, 0.22, 0), vector(-0.2, 0.78, 0), 0.115, skin);
  addCylinderBetween(legs, vector(0.19, 0.22, 0), vector(0.2, 0.78, 0), 0.115, skin);
  const leftBoot = roundedBox(0.28, 0.18, 0.48, 0.06, navy);
  leftBoot.position.set(-0.2, 0.1, 0.08);
  leftBoot.rotation.y = -0.08;
  legs.add(leftBoot);
  const rightBoot = roundedBox(0.28, 0.18, 0.48, 0.06, navy);
  rightBoot.position.set(0.2, 0.1, 0.08);
  rightBoot.rotation.y = 0.08;
  legs.add(rightBoot);
  group.add(legs);

  const skirt = mesh(new THREE.CylinderGeometry(0.43, 0.58, 0.42, 28), navy);
  skirt.position.y = 1.0;
  group.add(skirt);
  const skirtBand = mesh(new THREE.CylinderGeometry(0.44, 0.44, 0.055, 28), accentMaterial);
  skirtBand.position.y = 1.19;
  group.add(skirtBand);

  // Inner shirt, coat panels and blue trim.
  const torso = roundedBox(0.7, 0.66, 0.42, 0.11, dark, 4);
  torso.position.set(0, 1.43, 0.02);
  group.add(torso);
  const coatBack = roundedBox(0.9, 0.72, 0.22, 0.1, coat, 4);
  coatBack.position.set(0, 1.46, -0.15);
  group.add(coatBack);
  const leftCoat = roundedBox(0.31, 0.65, 0.13, 0.055, coatShadow, 3);
  leftCoat.position.set(-0.34, 1.46, 0.18);
  leftCoat.rotation.z = -0.08;
  group.add(leftCoat);
  const rightCoat = roundedBox(0.31, 0.65, 0.13, 0.055, coatShadow, 3);
  rightCoat.position.set(0.34, 1.46, 0.18);
  rightCoat.rotation.z = 0.08;
  group.add(rightCoat);
  const leftTrim = roundedBox(0.035, 0.58, 0.025, 0.012, accentMaterial, 2);
  leftTrim.position.set(-0.22, 1.46, 0.27);
  leftTrim.rotation.z = -0.08;
  group.add(leftTrim);
  const rightTrim = leftTrim.clone();
  rightTrim.position.x = 0.22;
  rightTrim.rotation.z = 0.08;
  group.add(rightTrim);

  const collar = roundedBox(0.29, 0.13, 0.08, 0.025, coat, 2);
  collar.position.set(0, 1.79, 0.28);
  group.add(collar);
  const tie = mesh(new THREE.ConeGeometry(0.09, 0.31, 4), accentMaterial);
  tie.position.set(0, 1.63, 0.32);
  tie.rotation.z = Math.PI;
  tie.scale.z = 0.45;
  group.add(tie);
  const tieClip = mesh(new THREE.SphereGeometry(0.045, 12, 8), gold);
  tieClip.position.set(0, 1.74, 0.34);
  group.add(tieClip);

  // Sleeves, hands and a small handheld game device.
  const arms = new THREE.Group();
  addCylinderBetween(arms, vector(-0.38, 1.7, 0.03), vector(-0.61, 1.23, 0.13), 0.14, coat);
  addCylinderBetween(arms, vector(0.38, 1.7, 0.03), vector(0.58, 1.25, 0.17), 0.14, coat);
  const leftHand = mesh(new THREE.SphereGeometry(0.12, 16, 12), skin);
  leftHand.position.set(-0.62, 1.18, 0.16);
  arms.add(leftHand);
  const rightHand = mesh(new THREE.SphereGeometry(0.12, 16, 12), skin);
  rightHand.position.set(0.59, 1.2, 0.2);
  arms.add(rightHand);
  const handheld = roundedBox(0.25, 0.17, 0.04, 0.025, navy, 3);
  handheld.position.set(-0.52, 1.2, 0.22);
  handheld.rotation.z = -0.18;
  arms.add(handheld);
  const screen = roundedBox(0.16, 0.09, 0.012, 0.012, accentMaterial, 2);
  screen.position.set(-0.52, 1.2, 0.246);
  screen.rotation.z = -0.18;
  arms.add(screen);
  group.add(arms);

  // Head and face.
  const head = new THREE.Group();
  head.name = "aris-preview-head";
  const face = mesh(new THREE.SphereGeometry(0.43, 32, 24), skin);
  face.scale.set(0.9, 1.08, 0.82);
  face.position.set(0, 2.1, 0.02);
  head.add(face);
  const neck = mesh(new THREE.CylinderGeometry(0.13, 0.15, 0.2, 16), skin);
  neck.position.set(0, 1.82, 0);
  head.add(neck);
  const leftEye = addEye(head, -0.145, eyeMaterial);
  const rightEye = addEye(head, 0.145, eyeMaterial);
  addBrow(head, -0.145, -0.06, hairShadow);
  addBrow(head, 0.145, 0.06, hairShadow);
  const mouth = mesh(new THREE.SphereGeometry(0.06, 16, 10), mouthMaterial);
  mouth.name = "aris-preview-mouth";
  mouth.position.set(0, 1.94, 0.405);
  mouth.scale.set(1.25, 0.15, 0.2);
  head.add(mouth);
  const nose = mesh(new THREE.SphereGeometry(0.032, 12, 8), skin);
  nose.position.set(0, 2.035, 0.408);
  nose.scale.set(0.85, 1.15, 0.7);
  head.add(nose);
  const cheekLeft = mesh(new THREE.SphereGeometry(0.042, 12, 8), new THREE.MeshBasicMaterial({ color: 0xffa9b4, transparent: true, opacity: 0.3 }));
  cheekLeft.position.set(-0.28, 1.99, 0.37);
  cheekLeft.scale.set(1.35, 0.55, 0.1);
  head.add(cheekLeft);
  const cheekRight = cheekLeft.clone();
  cheekRight.position.x = 0.28;
  head.add(cheekRight);
  group.add(head);

  // Long dark hair with teal tips, kept behind the face so the silhouette reads
  // clearly in a small transparent window.
  const hairGroup = new THREE.Group();
  const hairCap = mesh(new THREE.SphereGeometry(0.47, 32, 24), hairShadow);
  hairCap.position.set(0, 2.18, -0.16);
  hairCap.scale.set(1.02, 1.1, 0.82);
  hairGroup.add(hairCap);
  addHairRibbon(hairGroup, [
    [-0.39, 2.38], [-0.33, 2.48], [-0.19, 2.53], [-0.11, 2.42], [-0.16, 2.19], [-0.23, 2.04], [-0.28, 2.28],
  ], 0.055, hairShadow);
  addHairRibbon(hairGroup, [
    [-0.13, 2.49], [-0.02, 2.55], [0.08, 2.48], [0.13, 2.28], [0.08, 2.12], [0.02, 2.25],
  ], 0.05, hair);
  addHairRibbon(hairGroup, [
    [0.09, 2.49], [0.22, 2.51], [0.36, 2.4], [0.32, 2.2], [0.23, 2.08], [0.19, 2.3],
  ], 0.05, hair);
  const backStrands: [number, number][] = [
    [-0.34, -1], [-0.23, -1], [-0.11, -1], [0.11, 1], [0.23, 1], [0.34, 1],
  ];
  for (const [x, direction] of backStrands) {
    const start = vector(x, 2.38 - Math.abs(x) * 0.12, -0.14);
    const endX = x + direction * 0.09;
    addHairStrand(
      hairGroup,
      [start, vector(x + direction * 0.06, 1.95, -0.2), vector(endX, 1.42 - Math.abs(x) * 0.15, -0.22), vector(endX + direction * 0.02, 1.16, -0.16)],
      0.095 - Math.abs(x) * 0.03,
      hair,
      hairTip,
    );
  }
  // Side-swept bangs and a high side ponytail.
  addHairStrand(hairGroup, [vector(-0.3, 2.4, 0.14), vector(-0.27, 2.25, 0.31), vector(-0.18, 2.1, 0.34)], 0.052, hairShadow, hairTip);
  addHairStrand(hairGroup, [vector(-0.16, 2.44, 0.17), vector(-0.1, 2.27, 0.34), vector(-0.035, 2.13, 0.35)], 0.048, hair, hairTip);
  addHairStrand(hairGroup, [vector(0.01, 2.45, 0.15), vector(0.08, 2.28, 0.34), vector(0.13, 2.14, 0.34)], 0.044, hair, hairTip);
  addHairStrand(hairGroup, [vector(0.31, 2.36, -0.04), vector(0.54, 2.16, -0.1), vector(0.73, 1.84, -0.14), vector(0.7, 1.52, -0.1)], 0.12, hair, hairTip);
  addHairRibbon(hairGroup, [
    [-0.42, 2.25], [-0.52, 2.03], [-0.62, 1.66], [-0.57, 1.25], [-0.45, 1.08], [-0.43, 1.58], [-0.34, 1.98],
  ], 0.045, hairShadow);
  addHairRibbon(hairGroup, [
    [0.38, 2.23], [0.48, 2.02], [0.58, 1.66], [0.54, 1.24], [0.43, 1.06], [0.4, 1.58], [0.31, 2.0],
  ], 0.045, hair);
  const ponytailBand = mesh(new THREE.TorusGeometry(0.13, 0.027, 8, 18), accentMaterial);
  ponytailBand.position.set(0.48, 2.16, -0.1);
  ponytailBand.rotation.x = Math.PI / 2;
  hairGroup.add(ponytailBand);
  group.add(hairGroup);

  // Halo and small circuit-like markers, inspired by the character silhouette
  // but authored as new geometry rather than copied artwork.
  const halo = new THREE.Group();
  halo.name = "aris-preview-halo";
  halo.position.set(0, 2.18, -0.28);
  const haloRing = mesh(new THREE.TorusGeometry(0.56, 0.035, 10, 48), haloMaterial);
  halo.add(haloRing);
  const haloCore = mesh(new THREE.TorusGeometry(0.39, 0.012, 8, 36), gold);
  haloCore.rotation.z = Math.PI / 2;
  halo.add(haloCore);
  for (const [x, y, rotation] of [[-0.47, 0.13, -0.3], [0.47, 0.13, 0.3], [0, 0.53, 0]]) {
    const marker = roundedBox(0.08, 0.18, 0.03, 0.02, haloMaterial, 2);
    marker.position.set(x, y, 0);
    marker.rotation.z = rotation;
    halo.add(marker);
  }
  group.add(halo);

  // A compact Supernova-inspired back prop gives the silhouette a little more
  // character without making the desktop pet too wide.
  const prop = new THREE.Group();
  prop.position.set(0.58, 1.55, -0.27);
  prop.rotation.z = -0.28;
  const propBody = roundedBox(0.18, 0.82, 0.18, 0.045, coatShadow, 3);
  prop.add(propBody);
  const propCore = roundedBox(0.07, 0.62, 0.07, 0.02, accentMaterial, 2);
  propCore.position.z = 0.1;
  prop.add(propCore);
  const propEnd = mesh(new THREE.ConeGeometry(0.13, 0.22, 6), gold);
  propEnd.position.y = 0.5;
  propEnd.rotation.z = Math.PI;
  prop.add(propEnd);
  group.add(prop);

  // The preview is intentionally framed like a small desktop companion.
  group.scale.setScalar(0.5);
  group.position.y = 0.03;

  return { group, head, body: torso, leftEye, rightEye, mouth, halo, accentMaterial, haloMaterial };
}
