// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import type { Project } from "../../lib/types";
import { Metrics } from "./Metrics";
const mocks=vi.hoisted(()=>({save:vi.fn(),invalidate:vi.fn().mockResolvedValue(undefined)}));
vi.mock("../../lib/api",async(importOriginal)=>({...await importOriginal<typeof import("../../lib/api")>(),api:{setMetric:mocks.save}}));
vi.mock("../../lib/queries",()=>({useMetrics:()=>({isPending:false,isError:false,data:[]}),keys:{metrics:(slug:string)=>["project",slug,"metrics"]},queryClient:{invalidateQueries:mocks.invalidate}}));
const project={slug:"demo",status:"CREATED",role:"OWNER"} as Project;
afterEach(()=>{cleanup();vi.clearAllMocks()});
it("leaves missing KPIs explicitly unmeasured and lets a viewer read without editing",()=>{
  render(<MemoryRouter><Metrics project={{...project,role:"VIEWER"}}/></MemoryRouter>);
  expect(screen.getByText("No KPIs defined yet")).toBeTruthy();
  expect(screen.queryByRole("button",{name:"Add KPI"})).toBeNull();
  expect(screen.queryByRole("progressbar")).toBeNull();
});
it("saves an editable KPI definition without inventing a current measurement",async()=>{
  mocks.save.mockResolvedValue({});
  render(<MemoryRouter><Metrics project={project}/></MemoryRouter>);
  fireEvent.click(screen.getByRole("button",{name:"Add KPI"}));
  fireEvent.change(screen.getByLabelText("KPI name"),{target:{value:"Precision"}});
  fireEvent.change(screen.getByLabelText("Target",{exact:true}),{target:{value:"95"}});
  fireEvent.change(screen.getByLabelText("Unit"),{target:{value:"%"}});
  fireEvent.click(screen.getByRole("button",{name:"Save KPI"}));
  await waitFor(()=>expect(mocks.save).toHaveBeenCalledWith("demo",expect.any(String),expect.objectContaining({name:"Precision",target:95,current:null,evidence_id:null,observed_at:null}),0));
  await waitFor(()=>expect(screen.queryByRole("dialog")).toBeNull());
});
it("protects a changed definition from accidental dismissal",async()=>{
  render(<MemoryRouter><Metrics project={project}/></MemoryRouter>);
  fireEvent.click(screen.getByRole("button",{name:"Add KPI"}));
  fireEvent.change(screen.getByLabelText("KPI name"),{target:{value:"An unsaved target"}});
  fireEvent.click(screen.getByRole("button",{name:"Cancel"}));
  expect(screen.getByRole("heading",{name:"Discard KPI changes?"})).toBeTruthy();
  fireEvent.click(screen.getByRole("button",{name:"Discard changes"}));
  await waitFor(()=>expect(screen.queryByRole("dialog")).toBeNull());
  expect(mocks.save).not.toHaveBeenCalled();
});
