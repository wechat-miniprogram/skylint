import { Serializer } from "./interface";

export const serialize: Serializer<Record<"string", any>> = (node) => JSON.stringify(node, null, 2);
