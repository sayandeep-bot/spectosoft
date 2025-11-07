import { NavLink } from "react-router-dom";
import { LayoutDashboard, FolderKanban, BarChart2, LogOut } from "lucide-react";

export default function Sidebar() {
  return (
    <div className="h-screen w-64 bg-gray-900 text-white flex flex-col">
      <div className="text-2xl font-bold p-6 border-b border-gray-700">
        🧭 Dashboard
      </div>
      <nav className="flex-1 p-4 space-y-3">
        <NavLink to="/dashboard" className="flex items-center gap-3 p-2 hover:bg-gray-800 rounded">
          <LayoutDashboard /> Dashboard
        </NavLink>
        <NavLink to="/projects" className="flex items-center gap-3 p-2 hover:bg-gray-800 rounded">
          <FolderKanban /> Projects
        </NavLink>
        <NavLink to="/reports" className="flex items-center gap-3 p-2 hover:bg-gray-800 rounded">
          <BarChart2 /> Reports
        </NavLink>
      </nav>
      <button className="flex items-center gap-3 p-4 hover:bg-gray-800 border-t border-gray-700">
        <LogOut /> Logout
      </button>
    </div>
  );
}
