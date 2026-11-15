import { expect, test } from "bun:test";
import { pickContact, suggestTag } from "./ConversationDialogs";
test("finds the tag inside a pasted invite message", () => {
  expect(pickContact("Add me on Sotto: ada.mensah@relay.cmigl.org\n\nGet Sotto at https://www.vorynza.cloud/products/sotto")).toEqual({ kind: "tag", value: "ada.mensah@relay.cmigl.org" });
  expect(pickContact("  @Ada@relay.cmigl.org ")).toEqual({ kind: "tag", value: "@ada@relay.cmigl.org" });
  expect(pickContact("@ada")).toEqual({ kind: "tag", value: "@ada" });
  expect(pickContact("join me sotto://invite/abc123xyz please")).toEqual({ kind: "link", value: "sotto://invite/abc123xyz" });
  expect(pickContact("hello there")).toBeNull();
});
test("suggests a tag from a name", () => {
  expect(suggestTag("Ada Mensah")).toBe("ada.mensah");
  expect(suggestTag("Émile Zola")).toBe("emile.zola");
  expect(suggestTag("42")).toBe("");
});
