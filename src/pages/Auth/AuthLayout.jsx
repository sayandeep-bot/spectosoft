import React from "react";
import GridShape from "../../components/common/GridShape";
import { Link } from "react-router";
import ThemeTogglerTwo from "../../components/common/ThemeTogglerTwo";

export default function AuthLayout({
  children,
}) {
  return (
    <div className="relative p-6 bg-white z-1 dark:bg-[#002768] sm:p-0">
      <div className="relative flex flex-col justify-center w-full h-screen lg:flex-row dark:bg-[#002768] sm:p-0">
        {children}
        <div className="items-center hidden w-full h-full lg:w-1/2 bg-[#002768] dark:bg-white/5 lg:grid">
          <div className="relative flex items-center justify-center z-1">
            {/* <!-- ===== Common Grid Shape Start ===== --> */}
            <GridShape />
            <div className="flex flex-col items-center max-w-xs">
              <h1 className="mb-3 text-2xl font-bold text-white tracking-wide">
                Specto<span className="text-white/80">Soft</span>
              </h1>
              <p className="text-center text-white/70 text-sm leading-relaxed">
                Smart Monitoring Software for Teams and Businesses
              </p>

            </div>
          </div>
        </div>
        <div className="fixed z-50 hidden bottom-6 right-6 sm:block">
          <ThemeTogglerTwo />
        </div>
      </div>
    </div>
  );
}
