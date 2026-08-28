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
  z = -0.14,
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
  ribbon.position.z = z;
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
  const leftBoot = roundedBox(0.28, 0.18, 0.48, 0.06, white);
  leftBoot.position.set(-0.2, 0.1, 0.08);
  leftBoot.rotation.y = -0.08;
  legs.add(leftBoot);
  const rightBoot = roundedBox(0.28, 0.18, 0.48, 0.06, white);
  rightBoot.position.set(0.2, 0.1, 0.08);
  rightBoot.rotation.y = 0.08;
  legs.add(rightBoot);
  for (const [x, rotation] of [[-0.2, -0.08], [0.2, 0.08]] as const) {
    const sole = roundedBox(0.29, 0.045, 0.50, 0.018, navy, 3);
    sole.position.set(x, 0.012, 0.08);
    sole.rotation.y = rotation;
    legs.add(sole);
    const tongue = roundedBox(0.10, 0.035, 0.22, 0.012, accentMaterial, 2);
    tongue.position.set(x, 0.20, 0.18);
    tongue.rotation.y = rotation;
    legs.add(tongue);
  }
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
  for (const [x, rotation] of [[-0.34, -0.045], [0.34, 0.045]] as const) {
    const coatTail = roundedBox(0.28, 0.78, 0.13, 0.055, coatShadow, 3);
    coatTail.position.set(x, 1.22, -0.08);
    coatTail.rotation.z = rotation;
    group.add(coatTail);
    const coatTailEdge = roundedBox(0.035, 0.67, 0.025, 0.012, accentMaterial, 2);
    coatTailEdge.position.set(x + (x < 0 ? 0.105 : -0.105), 1.22, 0.0);
    coatTailEdge.rotation.z = rotation;
    group.add(coatTailEdge);
  }
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

  // Small pockets and shoulder stripes make the jacket read as clothing rather
  // than a pair of floating panels at the desktop-pet scale.
  for (const [x, rotation] of [[-0.28, -0.08], [0.28, 0.08]] as const) {
    const pocket = roundedBox(0.18, 0.12, 0.035, 0.018, dark, 3);
    pocket.position.set(x, 1.22, 0.28);
    pocket.rotation.z = rotation;
    group.add(pocket);
    const shoulderStripe = roundedBox(0.23, 0.035, 0.025, 0.012, accentMaterial, 2);
    shoulderStripe.position.set(x * 0.92, 1.72, 0.20);
    shoulderStripe.rotation.z = rotation;
    group.add(shoulderStripe);
  }

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
  for (const [x, rotation] of [[-0.62, -0.22], [0.59, 0.22]] as const) {
    const cuff = roundedBox(0.22, 0.12, 0.16, 0.035, navy, 3);
    cuff.position.set(x, 1.28, 0.14);
    cuff.rotation.z = rotation;
    arms.add(cuff);
  }
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

  // Long dark hair with cool gray/teal tips. The reference silhouette is driven
  // by two heavy side locks that fall past the jacket, plus a few thinner face
  // framing strands; keeping the broad masses behind the face makes this read
  // at small desktop-pet sizes too.
  const hairGroup = new THREE.Group();
  const hairCap = mesh(new THREE.SphereGeometry(0.47, 32, 24), hairShadow);
  hairCap.position.set(0, 2.18, -0.16);
  hairCap.scale.set(1.02, 1.1, 0.82);
  hairGroup.add(hairCap);
  for (const sign of [-1, 1] as const) {
    const side = sign < 0 ? "left" : "right";
    const deepSurface = sign < 0 ? hairShadow : hair;
    const longPath = [
      vector(0.27 * sign, 2.35, -0.13),
      vector(0.43 * sign, 2.12, -0.20),
      vector(0.54 * sign, 1.70, -0.24),
      vector(0.61 * sign, 1.22, -0.23),
      vector(0.55 * sign, 0.72, -0.20),
      vector(0.37 * sign, 0.30, -0.14),
    ];
    addHairStrand(hairGroup, longPath, 0.12, deepSurface, hairTip);
    addHairStrand(
      hairGroup,
      [
        vector(0.31 * sign, 2.28, -0.18),
        vector(0.49 * sign, 1.90, -0.25),
        vector(0.64 * sign, 1.43, -0.26),
        vector(0.62 * sign, 0.92, -0.23),
        vector(0.48 * sign, 0.48, -0.18),
      ],
      0.060,
      hair,
      hairTip,
    );
    addHairStrand(
      hairGroup,
      [
        vector(0.25 * sign, 2.34, 0.20),
        vector(0.36 * sign, 2.16, 0.30),
        vector(0.40 * sign, 1.83, 0.30),
        vector(0.33 * sign, 1.48, 0.26),
      ],
      0.055,
      deepSurface,
      hairTip,
    );
    const ponytailBand = mesh(new THREE.TorusGeometry(0.11, 0.021, 8, 20), accentMaterial);
    ponytailBand.name = `aris-preview-${side}-hair-band`;
    ponytailBand.position.set(0.42 * sign, 2.16, -0.18);
    ponytailBand.rotation.x = Math.PI / 2;
    hairGroup.add(ponytailBand);
  }
  // Layered front bangs, placed just in front of the face and kept narrow so
  // the cyan eyes remain the focal point.
  addHairRibbon(hairGroup, [
    [-0.34, 2.38], [-0.22, 2.48], [-0.10, 2.43], [-0.16, 2.12], [-0.25, 2.02], [-0.27, 2.28],
  ], 0.045, hairShadow, 0.32);
  addHairRibbon(hairGroup, [
    [-0.13, 2.48], [-0.01, 2.52], [0.08, 2.46], [0.03, 2.10], [-0.06, 2.00], [-0.08, 2.27],
  ], 0.042, hair, 0.33);
  addHairRibbon(hairGroup, [
    [0.08, 2.47], [0.20, 2.49], [0.31, 2.40], [0.25, 2.13], [0.16, 2.04], [0.17, 2.29],
  ], 0.042, hair, 0.32);
  group.add(hairGroup);

  // The reference halo reads as an open, segmented circuit frame rather than a
  // large circular ring. The gaps keep the face readable in a small window.
  const halo = new THREE.Group();
  halo.name = "aris-preview-halo";
  halo.position.set(0, 2.18, -0.31);
  const haloParts = [
    [0, 0.46, 0.58, 0.045],
    [-0.28, 0.28, 0.045, 0.30],
    [0.28, 0.28, 0.045, 0.30],
    [-0.22, 0.02, 0.16, 0.035],
    [0.22, 0.02, 0.16, 0.035],
  ] as const;
  for (const [x, y, width, height] of haloParts) {
    const part = roundedBox(width, height, 0.028, 0.014, haloMaterial, 3);
    part.position.set(x, y, 0);
    halo.add(part);
  }
  const haloCore = roundedBox(0.16, 0.045, 0.035, 0.012, gold, 2);
  haloCore.position.set(0, 0.46, 0.018);
  halo.add(haloCore);
  for (const [x, y] of [[-0.37, 0.41], [0.37, 0.41], [-0.32, -0.04], [0.32, -0.04]]) {
    const marker = roundedBox(0.045, 0.075, 0.032, 0.012, accentMaterial, 2);
    marker.position.set(x, y, 0.01);
    halo.add(marker);
  }
  group.add(halo);

  // A compact Supernova-inspired back prop keeps the railgun identity without
  // making the desktop pet as wide as the full illustration.
  const prop = new THREE.Group();
  prop.position.set(0.54, 1.55, -0.34);
  prop.rotation.z = -0.38;
  const propBody = roundedBox(0.24, 1.06, 0.20, 0.055, coatShadow, 3);
  prop.add(propBody);
  const propCore = roundedBox(0.09, 0.70, 0.07, 0.02, accentMaterial, 2);
  propCore.position.set(0, 0, 0.11);
  prop.add(propCore);
  const propEnd = mesh(new THREE.ConeGeometry(0.15, 0.25, 6), gold);
  propEnd.position.y = 0.49;
  propEnd.rotation.z = Math.PI;
  prop.add(propEnd);
  const propRail = roundedBox(0.10, 0.30, 0.10, 0.025, dark, 2);
  propRail.position.set(0, -0.53, 0);
  prop.add(propRail);
  group.add(prop);

  // The preview is intentionally framed like a small desktop companion.
  group.scale.setScalar(0.5);
  group.position.y = 0.03;

  return { group, head, body: torso, leftEye, rightEye, mouth, halo, accentMaterial, haloMaterial };
}
