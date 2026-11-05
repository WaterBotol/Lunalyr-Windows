import { createRoot } from "react-dom/client";
import { WavecoreCanvas } from "./components/scene/WavecoreCanvas";

createRoot(document.getElementById("wavecore")!).render(<WavecoreCanvas />);
