/// <reference types="vite/client" />

declare module "*.svg?raw" {
  const content: string;
  export default content;
}

declare module "*.svg?react" {
  import type { FunctionComponent, SVGProps } from "react";
  const component: FunctionComponent<SVGProps<SVGSVGElement>>;
  export default component;
}
