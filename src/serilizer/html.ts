import { DefaultTreeAdapterMap, serialize as rawSerilize } from "parse5";
import { Serializer } from "./interface";

type ParentNode = Parameters<typeof rawSerilize<DefaultTreeAdapterMap>>[0]

export const serialize: Serializer<ParentNode> = (ast) => rawSerilize(ast);