export default function Navbar() {
  return (
    <div className="flex justify-between items-center bg-white p-4 shadow-sm">
      <h2 className="text-xl font-semibold">Dashboard Overview</h2>
      <div className="flex items-center gap-3">
        <input
          type="text"
          placeholder="Search..."
          className="border rounded px-3 py-1 text-sm"
        />
        <img
          src="https://i.pravatar.cc/40"
          alt="User Avatar"
          className="w-8 h-8 rounded-full"
        />
      </div>
    </div>
  );
}
