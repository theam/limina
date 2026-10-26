// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import type { Artifact } from "../../lib/types";
import { KnowledgeHome } from "./KnowledgeHome";
const mocks=vi.hoisted(()=>({query:vi.fn()}));
vi.mock("../../components/shell/Crumbs",()=>({useCrumbs:()=>{}}));
vi.mock("./ProjectContext",()=>({useProjectContext:()=>({slug:"demo"})}));
vi.mock("../../lib/queries",()=>({useProjectStatus:()=>({data:{project:{name:"Demo"},knowledge:{H:{PROPOSED:1000}}}}),useKnowledgeList:(...args:unknown[])=>mocks.query(...args)}));
const artifacts=Array.from({length:1000},(_,index)=>({id:`H${String(index+1).padStart(3,"0")}`,kind:"H",title:`Complete readable research title ${index+1}`,status:"PROPOSED",content:{statement:`Specific evidence for note ${index+1}`},created_at:"2026-10-21T12:00:00Z",tags:[],updated_at:"2026-10-21T12:00:00Z"}) as Artifact);
function Location(){return <output aria-label="Current URL">{useLocation().search}</output>}
function setup(){
  mocks.query.mockImplementation((_slug:string,filters:{query?:string;cursor?:string;limit:number})=>{
    const all=filters.query?artifacts.filter(item=>item.id.toLowerCase()===filters.query?.toLowerCase()):artifacts;
    const offset=filters.cursor?Number(filters.cursor):0;
    return {isPending:false,isError:false,data:{items:all.slice(offset,offset+filters.limit),total:all.length,next_cursor:all.length>offset+filters.limit?String(offset+filters.limit):null}};
  });
  return render(<MemoryRouter initialEntries={["/projects/demo/knowledge"]}><Routes><Route path="/projects/demo/knowledge" element={<><KnowledgeHome/><Location/></>}/></Routes></MemoryRouter>);
}
afterEach(()=>{cleanup();vi.clearAllMocks()});
it("renders one bounded page and preserves navigation to the previous page",async()=>{
  setup();
  expect(screen.getAllByRole("article")).toHaveLength(25);
  expect(screen.queryByRole("heading",{name:"Complete readable research title 26"})).toBeNull();
  fireEvent.click(screen.getByRole("link",{name:/Next page/}));
  await waitFor(()=>expect(screen.getByRole("heading",{name:"Complete readable research title 26"})).toBeTruthy());
  expect(screen.getAllByRole("article")).toHaveLength(25);
  fireEvent.click(screen.getByRole("link",{name:/Previous page/}));
  expect(screen.getByRole("heading",{name:"Complete readable research title 1"})).toBeTruthy();
});
it("searches an off-page ID through the server query and stores it in the URL",async()=>{
  setup();
  fireEvent.change(screen.getByRole("searchbox",{name:"Search project knowledge"}),{target:{value:"H999"}});
  await waitFor(()=>expect(screen.getByRole("heading",{name:"Complete readable research title 999"})).toBeTruthy());
  expect(screen.getAllByRole("article")).toHaveLength(1);
  expect(screen.getByLabelText("Current URL").textContent).toContain("q=H999");
  expect(mocks.query).toHaveBeenCalledWith("demo",expect.objectContaining({query:"H999",limit:25}),true);
  fireEvent.click(screen.getByRole("button",{name:"Clear filters"}));
  expect(screen.getByLabelText("Current URL").textContent).toBe("");
});
