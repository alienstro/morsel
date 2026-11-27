import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { windowLabel } from "./lib/window";
import { ReminderApp } from "./reminder/ReminderApp";
import "./styles/fonts";
import "./styles/tokens.css";

const label = windowLabel();
document.documentElement.dataset.window = label;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {label === "reminder" ? <ReminderApp /> : <App />}
  </React.StrictMode>,
);
