import { lazy, Suspense } from "react";
import { ChatPanel } from "./components/ChatPanel";

const PetWindow = lazy(() =>
  import("./components/PetWindow").then((module) => ({ default: module.PetWindow })),
);

export function App({ windowLabel }: { windowLabel: string }) {
  return windowLabel === "pet" ? (
    <Suspense fallback={<main className="pet-shell" />}>
      <PetWindow />
    </Suspense>
  ) : (
    <ChatPanel />
  );
}
