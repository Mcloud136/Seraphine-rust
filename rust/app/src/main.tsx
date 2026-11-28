import React from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import ReactDOM from "react-dom/client";
import App from "./App";
import OpggWindow from "./OpggWindow";
import "./styles.css";

const isOpggWindow = getCurrentWindow().label === "opgg";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {isOpggWindow ? <OpggWindow /> : <App />}
  </React.StrictMode>,
);