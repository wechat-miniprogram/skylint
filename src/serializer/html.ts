import rawSerilize from "dom-serializer";
import { ParentNode } from "domhandler";
import { Serializer } from "./interface";

export const serialize: Serializer<ParentNode> = (ast) => rawSerilize(ast);
