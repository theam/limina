// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { Overview } from "./Overview";
const mocks=vi.hoisted(()=>({notes:vi.fn()}));
vi.mock("../../components/shell/Crumbs",()=>({useCrumbs:()=>{}}));
vi.mock("../../components/project/Metrics",()=>({Metrics:()=> <section aria-label="Measured KPIs"/>}));
vi.mock("./ProjectContext",()=>({useProjectContext:()=>({slug:"demo"}),useFeedState:()=>({loaded:true,events:[]})}));
vi.mock("../../lib/queries",()=>({
  useProjectStatus:()=>({data:{project:{slug:"demo",name:"A readable full project title",role:"OWNER",runtime:"codex",status:"CREATED",mission:"An extensive mission that belongs in its detail view",success_criteria:"95% precision",current_objective:"A complete objective with a final readable condition.",next_step:"Measure the held-out test split.",blocker:"None"},knowledge:{H:{PROPOSED:37}}}}),
  usePreflight:()=>({data:{checks:[]}}),useKnowledgeList:(...args:unknown[])=>{mocks.notes(...args);return {isPending:false,isError:false,data:{items:[]}}},
  useRuns:()=>({isPending:false,isError:false,data:{items:[]}}),
}));
afterEach(cleanup);
it("prioritizes mission outcomes and access to detail without duplicate metadata or activity scores",()=>{
  render(<MemoryRouter><Overview/></MemoryRouter>);
  expect(screen.getByRole("heading",{level:1,name:"A readable full project title"})).toBeTruthy();
  expect(screen.getByRole("link",{name:/Read mission and success criteria/}).getAttribute("href")).toContain("CHALLENGE");
  expect(screen.queryByText("An extensive mission that belongs in its detail view")).toBeNull();
  expect(screen.queryByText(/owner/i)).toBeNull();
  expect(screen.queryByText("Draft")).toBeNull();
  expect(screen.queryByText("37")).toBeNull();
  expect(screen.getByText("Read full current objective")).toBeTruthy();
  expect(mocks.notes).toHaveBeenCalledWith("demo",{kind:"F",limit:3});
});
