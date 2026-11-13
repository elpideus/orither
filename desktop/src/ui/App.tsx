import { usePeers } from "../core/link";
import { Pair } from "./Pair";
import { Shell } from "./Shell";

export function App() {
  const peers = usePeers();
  if (peers === null) return null;
  return peers.length === 0 ? <Pair /> : <Shell />;
}
