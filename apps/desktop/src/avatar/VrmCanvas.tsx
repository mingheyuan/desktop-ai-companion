import { useEffect, useRef } from "react";
import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { VRMLoaderPlugin, VRMUtils, type VRM } from "@pixiv/three-vrm";
import type { AvatarSemanticState } from "../contracts";

interface VrmCanvasProps {
  state: AvatarSemanticState;
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

export function VrmCanvas({ state, modelUrl }: VrmCanvasProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const stateRef = useRef(state);
  stateRef.current = state;

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(30, 1, 0.1, 20);
    camera.position.set(0, 1.25, 2.4);
    camera.lookAt(0, 1.15, 0);

    const renderer = new THREE.WebGLRenderer({ alpha: true, antialias: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    host.appendChild(renderer.domElement);

    scene.add(new THREE.HemisphereLight(0xdcecff, 0x18213c, 2.4));
    const key = new THREE.DirectionalLight(0xffffff, 2.2);
    key.position.set(1.5, 2.5, 2);
    scene.add(key);

    const fallbackMaterial = new THREE.MeshStandardMaterial({ color: stateColors[state], roughness: 0.32, metalness: 0.08 });
    const fallback = new THREE.Group();
    const head = new THREE.Mesh(new THREE.SphereGeometry(0.34, 32, 32), fallbackMaterial);
    head.position.y = 1.45;
    const body = new THREE.Mesh(new THREE.CapsuleGeometry(0.32, 0.65, 8, 24), fallbackMaterial);
    body.position.y = 0.72;
    fallback.add(head, body);
    scene.add(fallback);

    let vrm: VRM | null = null;
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
    const render = (now = 0) => {
      frame = requestAnimationFrame(render);
      if (document.hidden) return;
      const semantic = stateRef.current;
      const minimumFrameTime = semantic === "sleeping" ? 66 : semantic === "idle" ? 33 : 16;
      if (now - lastRenderAt < minimumFrameTime) return;
      lastRenderAt = now;
      const delta = Math.min(clock.getDelta(), 0.05);
      const elapsed = clock.elapsedTime;
      fallbackMaterial.color.setHex(stateColors[semantic]);
      fallback.position.y = Math.sin(elapsed * (semantic === "speaking" ? 4 : 1.5)) * 0.025;
      fallback.rotation.y = Math.sin(elapsed * 0.55) * 0.08;
      if (vrm) {
        vrm.expressionManager?.setValue("happy", semantic === "ready" ? 0.55 : 0);
        vrm.expressionManager?.setValue("surprised", semantic === "needs_input" ? 0.35 : 0);
        vrm.expressionManager?.setValue("aa", semantic === "speaking" ? 0.22 + Math.sin(elapsed * 12) * 0.12 : 0);
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
        if (object instanceof THREE.Mesh) object.geometry.dispose();
      });
      fallbackMaterial.dispose();
      renderer.dispose();
      renderer.domElement.remove();
    };
  }, [modelUrl]);

  return <div className="vrm-canvas" ref={hostRef} aria-label={`角色状态：${state}`} />;
}
