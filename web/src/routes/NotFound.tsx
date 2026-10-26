import { Link } from "react-router";
import { useCrumbs } from "../components/shell/Crumbs";
import { Empty } from "../components/ui/Empty";

export function NotFound() {
  useCrumbs([{ label: "Not found" }]);
  return (
    <div className="page page-narrow">
      <Empty
        title="This page doesn’t exist"
        action={
          <Link to="/projects" className="btn">
            Go to projects
          </Link>
        }
      >
        The link may be outdated, or the project may have been archived.
      </Empty>
    </div>
  );
}
