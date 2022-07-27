import { generate, CssNode } from "css-tree";
import { Serializer } from "./interface";

export const serialize: Serializer<CssNode> = (node: CssNode) => generate(node);
