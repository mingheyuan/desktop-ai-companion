import { useEffect, useRef } from "react";
import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { VRMHumanBoneName, VRMLoaderPlugin, VRMUtils, type VRM } from "@pixiv/three-vrm";
import type { AvatarSemanticState } from "../contracts";
import { createArisPreview } from "./arisPreview";
import { createArisBlockPreview } from "./arisBlockPreview";
import { createArisBbmodelPreview } from "./arisBbmodelPreview";

interface VrmCanvasProps {
  state: AvatarSemanticState;
  mouthLevel: number;
  modelUrl?: string;
}

const stateColors: Record<AvatarSemanticState, number> = {
  idle: 0x79a9ff,
  listening: 0x70e1c1,
  thinking: 0xb99cff,
  speaking: 0x68c7ff,
  sleeping: 0x61709a,
  needs_input: 0xffc86b,
  ready: 0x8cf0d1,
  recovering: 0xffa86b,
  error: 0xff6f91,
};

export function VrmCanvas({ state, mouthLevel, modelUrl }: VrmCanvasProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const stateRef = useRef(state);
  const mouthLevelRef = useRef(mouthLevel);
  stateRef.current = state;
  mouthLevelRef.current = mouthLevel;

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const avatarMode = new URLSearchParams(window.location.search).get("avatar");
    const showReferenceWeapon = new URLSearchParams(window.location.search).get("weapon") !== "off";
    const useReferencePreview = avatarMode === "block" || avatarMode === "figura" || avatarMode === "reference";
    const useBlockPreview = avatarMode === "procedural";
    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(30, 1, 0.1, 20);
    camera.position.set(0, 0.95, 3.45);
    camera.lookAt(0, useReferencePreview ? 0.8 : useBlockPreview ? 0.72 : 0.9, 0);

    const renderer = new THREE.WebGLRenderer({ alpha: true, antialias: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.toneMappingExposure = 1.12;
    renderer.domElement.style.width = "100%";
    renderer.domElement.style.height = "100%";
    renderer.domElement.style.display = "block";
    host.appendChild(renderer.domElement);

    scene.add(new THREE.HemisphereLight(0xdcecff, 0x18213c, 2.4));
    const key = new THREE.DirectionalLight(0xffffff, 2.2);
    key.position.set(1.5, 2.5, 2);
    scene.add(key);
    const rim = new THREE.PointLight(0x66dfff, 1.8, 4.5);
    rim.position.set(-1.2, 1.8, 1.4);
    scene.add(rim);

    const preview = useReferencePreview
      ? createArisBbmodelPreview()
      : useBlockPreview
        ? createArisBlockPreview()
        : createArisPreview();
    const fallback = preview.group;
    scene.add(fallback);
    if ("weapon" in preview && preview.weapon) preview.weapon.visible = showReferenceWeapon;

    let vrm: VRM | null = null;
    let vrmHead: THREE.Object3D | null = null;
    let vrmSpine: THREE.Object3D | null = null;
    let vrmHeadBase: THREE.Euler | null = null;
    let vrmSpineBase: THREE.Euler | null = null;
    let disposed = false;
    if (modelUrl) {
      const loader = new GLTFLoader();
      loader.register((parser) => new VRMLoaderPlugin(parser));
      loader.load(modelUrl, (gltf) => {
        const loaded = gltf.userData.vrm as VRM | undefined;
        if (!loaded) return;
        VRMUtils.removeUnnecessaryVertices(gltf.scene);
        VRMUtils.combineSkeletons(gltf.scene);
        VRMUtils.rotateVRM0(loaded);
        if (disposed) {
          VRMUtils.deepDispose(loaded.scene);
          return;
        }
        loaded.scene.traverse((object) => {
          object.frustumCulled = false;
        });
        const bounds = new THREE.Box3().setFromObject(loaded.scene);
        const size = bounds.getSize(new THREE.Vector3());
        if (size.y > 0) loaded.scene.scale.setScalar(1.7 / size.y);
        const fitted = new THREE.Box3().setFromObject(loaded.scene);
        const center = fitted.getCenter(new THREE.Vector3());
        loaded.scene.position.x -= center.x;
        loaded.scene.position.y -= fitted.min.y;
        vrm = loaded;
        vrmHead = loaded.humanoid?.getNormalizedBoneNode(VRMHumanBoneName.Head) ?? null;
        vrmSpine = loaded.humanoid?.getNormalizedBoneNode(VRMHumanBoneName.Spine) ?? null;
        vrmHeadBase = vrmHead?.rotation.clone() ?? null;
        vrmSpineBase = vrmSpine?.rotation.clone() ?? null;
        fallback.visible = false;
        scene.add(loaded.scene);
      }, undefined, () => {
        fallback.visible = true;
      });
    }

    const clock = new THREE.Clock();
    let frame = 0;
    const resize = () => {
      const { width, height } = host.getBoundingClientRect();
      if (!width || !height) return;
      renderer.setSize(width, height, false);
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
    };
    const observer = new ResizeObserver(resize);
    observer.observe(host);
    resize();

    let lastRenderAt = 0;
    let smoothedMouth = 0;
    const render = (now = 0) => {
      frame = requestAnimationFrame(render);
      if (document.hidden) return;
      const semantic = stateRef.current;
      const minimumFrameTime = semantic === "sleeping" ? 66 : semantic === "idle" ? 33 : 16;
      if (now - lastRenderAt < minimumFrameTime) return;
      lastRenderAt = now;
      const delta = Math.min(clock.getDelta(), 0.05);
      const elapsed = clock.elapsedTime;
      const listeningWeight = semantic === "listening" ? 1 : 0;
      const thinkingWeight = semantic === "thinking" ? 1 : 0;
      const speakingWeight = semantic === "speaking" ? 1 : 0;
      const sleepingWeight = semantic === "sleeping" ? 1 : 0;
      const targetMouth = speakingWeight ? THREE.MathUtils.clamp(mouthLevelRef.current, 0, 1) : 0;
      smoothedMouth = THREE.MathUtils.damp(smoothedMouth, targetMouth, targetMouth > smoothedMouth ? 22 : 14, delta);
      const blinkPhase = elapsed % 4.6;
      const blink = sleepingWeight ? 0.82 : blinkPhase > 4.34 ? Math.sin(((blinkPhase - 4.34) / 0.26) * Math.PI) : 0;
      const headPitch = sleepingWeight * 0.16 + listeningWeight * -0.035 + speakingWeight * Math.sin(elapsed * 3.2) * 0.018;
      const headYaw = listeningWeight * Math.sin(elapsed * 1.2) * 0.075 + thinkingWeight * 0.11 + speakingWeight * Math.sin(elapsed * 2.1) * 0.04;
      const headRoll = listeningWeight * -0.065 + thinkingWeight * 0.085;
      const breath = Math.sin(elapsed * (sleepingWeight ? 0.8 : 1.55));
      preview.accentMaterial.color.setHex(stateColors[semantic]);
      preview.haloMaterial.color.setHex(stateColors[semantic] === stateColors.error ? 0xff93a9 : stateColors[semantic]);
      fallback.position.y = Math.sin(elapsed * (semantic === "speaking" ? 4 : 1.5)) * 0.025;
      fallback.rotation.y = (useReferencePreview ? 0.16 : 0) + Math.sin(elapsed * 0.55) * 0.08;
      preview.head.rotation.set(headPitch, headYaw, headRoll);
      preview.body.scale.y = 1 + breath * 0.012;
      preview.leftEye.scale.y = preview.rightEye.scale.y = Math.max(0.08, 1 - blink);
      preview.mouth.scale.y = 0.15 * (1 + smoothedMouth * 5);
      preview.halo.rotation.y = Math.sin(elapsed * 0.45) * 0.06;
      if (vrm) {
        vrm.expressionManager?.setValue("happy", semantic === "ready" ? 0.55 : 0);
        vrm.expressionManager?.setValue("surprised", semantic === "needs_input" ? 0.35 : 0);
        vrm.expressionManager?.setValue("blink", blink);
        vrm.expressionManager?.setValue("aa", smoothedMouth);
        if (vrmHead && vrmHeadBase) {
          vrmHead.rotation.set(vrmHeadBase.x + headPitch, vrmHeadBase.y + headYaw, vrmHeadBase.z + headRoll, vrmHeadBase.order);
        }
        if (vrmSpine && vrmSpineBase) {
          vrmSpine.rotation.set(vrmSpineBase.x + breath * 0.008, vrmSpineBase.y, vrmSpineBase.z + thinkingWeight * -0.025, vrmSpineBase.order);
        }
        vrm.update(delta);
      }
      renderer.render(scene, camera);
    };
    render();

    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      observer.disconnect();
      if (vrm) {
        scene.remove(vrm.scene);
        VRMUtils.deepDispose(vrm.scene);
      }
      fallback.traverse((object) => {
        if (!(object instanceof THREE.Mesh)) return;
        object.geometry.dispose();
        if (Array.isArray(object.material)) object.material.forEach((surface) => surface.dispose());
        else object.material.dispose();
      });
      renderer.dispose();
      renderer.domElement.remove();
    };
  }, [modelUrl]);

  return <div className="vrm-canvas" ref={hostRef} aria-label={`角色状态：${state}`} />;
}
