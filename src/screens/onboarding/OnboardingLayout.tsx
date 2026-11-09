import type { ReactNode } from "react";
import painting from "../../assets/sisley-loire.jpg";
import "./onboarding.css";

type Props = { step: 1 | 2 | 3; title: string; lead: string; children: ReactNode; footer: ReactNode };

/** The frame of the first-run screens: the painting on the left and the step on the right. */
export function OnboardingLayout({ step, title, lead, children, footer }: Props) {
  return (
    <div className="onboard">
      <div className="titlebar" data-tauri-drag-region />
      <div className="onboard-body">
        <figure className="onboard-art">
          <img src={painting} alt="A misty river bank with bare trees and a moored boat" />
          <figcaption className="serif">Alfred Sisley, The Loire, 1896</figcaption>
        </figure>
        <main className="onboard-main">
          <div className="onboard-content">
            <div className="muted small">Step {step} of 3</div>
            <div className="onboard-head">
              <h1 className="serif">{title}</h1>
              <p className="muted">{lead}</p>
            </div>
            {children}
            <div className="onboard-footer">{footer}</div>
          </div>
        </main>
      </div>
    </div>
  );
}
