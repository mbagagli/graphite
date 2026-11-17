import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { TopicControls } from "./TopicControls";
import { demoGraph } from "../../adapters/demo/demoData";
import type { Topic } from "../../domain/research";

const render = (topics: Topic[]) =>
  renderToStaticMarkup(
    <TopicControls
      graph={demoGraph}
      topics={topics}
      grouping="relations"
      onGroupingChange={() => {}}
      onPrepare={() => {}}
    />,
  );

describe("topic preparation states", () => {
  it("requests semantic preparation and disables grouping on a fresh start", () => {
    const html = render([]);
    expect(html).toContain("Semantic indexing is required");
    expect(html).toContain("Prepare library for AI");
    expect(html).toContain('disabled=""');
    expect(html).not.toContain("primary topic tag");
  });
  it("explains abstention rather than implying fabricated categories", () => {
    const html = render([
      {
        id: "uncategorized",
        name: "No clear semantic groups",
        shortName: "Unassigned",
        color: "#929a9c",
        origin: "uncategorized",
        analyzed: 60,
        eligible: 60,
      },
    ]);
    expect(html).toContain("No clear semantic groups were found");
    expect(html).toContain("60 / 60 papers analyzed");
    expect(html).toContain('disabled=""');
  });
  it("identifies content-based maps without changing relationship controls", () => {
    expect(
      render([
        {
          id: "example",
          name: "Surface waves",
          shortName: "Surface waves",
          color: "#5dcfd1",
          origin: "semantic-cluster",
          analyzed: 30,
          eligible: 60,
        },
      ]),
    ).toContain("Automatic similarity groups");
  });
});
