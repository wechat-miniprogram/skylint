import * as domhandler from 'domhandler';
import { Document, Text, ProcessingInstruction, Comment, Element, CDATA, Node as Node$1 } from 'domhandler';
import { CssNode, WalkContext } from 'css-tree';
import parseJSON, { ArrayNode, IdentifierNode, LiteralNode, ObjectNode, PropertyNode, ValueNode } from 'json-to-ast';

interface PatchLocation {
    start: number;
    end: number;
    path: string;
}
declare const enum PatchStatus {
    Pending = 0,
    Applied = 1,
    Failed = 2
}
interface Patch {
    name: string;
    loc: PatchLocation;
    patchedStr: string;
    status: PatchStatus;
}

type WalkerReturnType = boolean | void;
interface DefaultWalkerContext<T> {
    parent: T;
}
type Walker<T, K = null> = K extends Object ? (node: T, callback: (node: T, ctx: K) => WalkerReturnType, ctx: K) => WalkerReturnType : (node: T, callback: (node: T) => WalkerReturnType) => WalkerReturnType;

interface NodeTypeMap {
    Root: Document;
    Text: Text;
    Directive: ProcessingInstruction;
    Comment: Comment;
    Script: Element;
    Style: Element;
    Tag: Element;
    CDATA: CDATA;
    Doctype: ProcessingInstruction;
}
type HTMLWalker = Walker<Node$1, DefaultWalkerContext<Node$1>>;

type CSSWalker = Walker<CssNode, WalkContext>;

type Node = ArrayNode | IdentifierNode | LiteralNode | ObjectNode | PropertyNode;
type JSONWalker = Walker<Node, DefaultWalkerContext<Node>>;

declare enum RuleLevel {
    Verbose = 0,
    Info = 1,
    Warn = 2,
    Error = 3
}
interface BasicLocation {
    path: string | null;
}
interface LocationLnColBased extends BasicLocation {
    startLn: number;
    endLn: number;
    startCol: number;
    endCol: number;
}
interface LocationIndexBased extends BasicLocation {
    startIndex: number;
    endIndex: number;
}
type SourceCodeLocation = LocationIndexBased | LocationLnColBased;
interface RuleResultItem {
    name: string;
    description?: string;
    advice?: string;
    patchHint?: string;
    loc?: SourceCodeLocation;
    level: RuleLevel;
    fixable?: boolean;
    withCodeFrame?: boolean;
}
declare const enum RuleType {
    Unknown = 0,
    WXML = 1,
    WXSS = 2,
    Node = 3,
    JSON = 4
}
interface HookType {
    [RuleType.WXML]: Parameters<HTMLWalker>[1];
    [RuleType.WXSS]: Parameters<CSSWalker>[1];
    [RuleType.JSON]: Parameters<JSONWalker>[1];
    [RuleType.Node]: any;
    [RuleType.Unknown]: never;
}
interface Hooks<T extends RuleType> {
    before?: () => void;
    onVisit?: HookType[T];
    after?: () => void;
}
interface RuleBasicInfo<T extends RuleType = RuleType> {
    name: string;
    type: T;
    level: RuleLevel;
}
interface Rule<T extends RuleType = RuleType> extends Hooks<T>, RuleBasicInfo {
    get results(): RuleResultItem[];
    get patches(): Patch[];
    get astPatches(): Function[];
    clear(): void;
}
interface RuleContext<T extends RuleType, K> {
    lifetimes(hooks: Hooks<T>): void;
    addResult(...results: RuleResultItem[]): void;
    addResultWithPatch(result: RuleResultItem, patch: QuickPatch): void;
    addPatch(...patches: Omit<Patch, "status">[]): void;
    addASTPatch(...patches: Function[]): void;
    getRelatedWXMLFilename(): string | undefined;
    getRelatedWXMLAst(): Document | null;
    env: K;
}
type QuickPatch = Pick<Patch, "loc" | "patchedStr">;
type RuleBasicInfoWithOptionalLevel<T extends RuleType> = Pick<RuleBasicInfo<T>, "name" | "type">;
declare const defineRule: <Env extends BasicParseEnv, T extends RuleType = RuleType>(info: RuleBasicInfoWithOptionalLevel<T>, init: (ctx: RuleContext<T, Env>) => void) => (env: Env) => Rule<T>;

interface BasicParseEnv {
    path: string;
    astMap?: Map<string, any>;
}
interface IParseOptions<T extends BasicParseEnv> {
    wxml?: string;
    wxss?: string;
    json?: string;
    Rules?: ((env: T) => Rule<any>)[];
    env: T;
    astWXML?: NodeTypeMap["Root"];
    astWXSS?: CssNode;
    astJSON?: ValueNode;
}
declare const parse: <T extends BasicParseEnv>(options: IParseOptions<T>) => {
    astWXML: domhandler.Document | undefined;
    astWXSS: CssNode | undefined;
    astJSON: parseJSON.ValueNode | undefined;
    ruleResults: {
        name: string;
        level: RuleLevel;
        results: RuleResultItem[];
        patches: Patch[];
    }[];
};

export { defineRule, parse };
