import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import AutoPublish from "./components/AutoPublish";
import "./styles.css";

// "#autopublish=<feature>&group=<menu group>": the engine publishing on its own (자동 작업).
const params = new URLSearchParams(location.hash.slice(1));
const auto = params.has("autopublish") ? params : null;

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {auto ? <AutoPublish feature={auto.get("autopublish")!} group={auto.get("group")} /> : <App />}
  </StrictMode>,
);
