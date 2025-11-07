import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useNavigate } from "react-router-dom";

import { Eye, EyeOff } from 'lucide-react'
// import { ChevronLeftIcon, EyeCloseIcon, EyeIcon } from "../components/icons";
import AuthLayout from "./AuthLayout";
import Label from "../../components/form/Label";
import Input from "../../components/form/input/InputField";
import Checkbox from "../../components/form/input/Checkbox";
import Button from "../../components/ui/button/Button";
import { Link } from "react-router-dom";
import axios from 'axios';

export default function Login() {
  const [body, setBody] = useState({
    email: "",
    password: ""
  });
  const [showPassword, setShowPassword] = useState(false);
  const [isChecked, setIsChecked] = useState(false);
  const navigate = useNavigate();

  async function handleLogin(e) {
    e.preventDefault();

    // try {
    //   const res = await axios.post("http://127.0.0.1:3005/login", body, { "accept": "application/json" });
    //   console.log("Login response:", res.data);

    //   if (res.data?.status == 200) {
    //     // Example: Navigate to dashboard
    //     window.localStorage.setItem("token", res.data?.token);
    //     window.localStorage.setItem("name", res.data?.user?.name);
    //     window.localStorage.setItem("email", res.data?.user?.email);
    //     window.location.href = "/";
    //   } else {
    //     alert(res.data?.message || "Invalid credentials");
    //   }
    // } catch (error) {
    //   console.error("Login error:", error);
    //   alert("Error connecting to server");
    // }

    window.localStorage.setItem("token", "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJlbWFpbCI6InNheWFuZGVlcEBrbGl6b3MuY29tIiwibmFtZSI6IlNheWFuZGVlcCIsInJvbGUiOiJBZG1pbiIsImV4cCI6MTc2MjMzMzk5NX0.kQt2w6lEd2OMTlYajbSzCZtcktdTubUrsKNgi0VGsEk");
    window.location.href = "/";

  }

  return (
    <AuthLayout>
      <div className="flex flex-col flex-1">
        <div className="flex flex-col justify-center item-center flex-1 w-full max-w-md mx-auto">
          <div>
              <Link to="/" className="flex justify-center item-center ">
                <img
                  width={131}
                  height={28}
                  src="images/logo/spectosoft.jpg"
                  alt="Logo"
                />
              </Link>
            <div className="mb-5 sm:mb-8">
              <h1 className="mb-2 font-semibold text-gray-800 text-title-sm dark:text-white/90 sm:text-title-md">
                Sign In
              </h1>
              <p className="text-sm text-gray-500 dark:text-gray-400">
                Enter your email and password to sign in!
              </p>
            </div>
            <div>
              <form onSubmit={handleLogin}>
                <div className="space-y-6">
                  <div>
                    <Label>
                      Email <span className="text-error-500">*</span>{" "}
                    </Label>
                    <Input
                      value={body.email}
                      onChange={(e) => setBody({ ...body, email: e.target.value })}
                      placeholder="info@gmail.com"
                    />
                  </div>
                  <div>
                    <Label>
                      Password <span className="text-error-500">*</span>{" "}
                    </Label>
                    <div className="relative">
                      <Input
                        type={showPassword ? "text" : "password"}
                        value={body.password}
                        onChange={(e) => setBody({ ...body, password: e.target.value })}
                        placeholder="Enter your password"
                      />
                      <span
                        onClick={() => setShowPassword(!showPassword)}
                        className="absolute z-30 -translate-y-1/2 cursor-pointer right-4 top-1/2"
                      >
                        {showPassword ? (
                          <Eye className="fill-gray-500 dark:fill-gray-400 size-5" />
                        ) : (
                          <EyeOff className="fill-gray-500 dark:fill-gray-400 size-5" />
                        )}
                      </span>
                    </div>
                  </div>
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-3">
                      <Checkbox checked={isChecked} onChange={setIsChecked} />
                      <span className="block font-normal text-gray-700 text-theme-sm dark:text-gray-400">
                        Keep me logged in
                      </span>
                    </div>
                    <Link
                      to="#!"
                      className="text-sm text-brand-500 hover:text-brand-600 dark:text-brand-400"
                    >
                      Forgot password?
                    </Link>
                  </div>
                  <div>
                    <Button className="w-full" size="sm" onClick={handleLogin}>
                      Sign in
                    </Button>
                  </div>
                </div>
              </form>

              <div className="mt-5">
                <p className="text-sm font-normal text-center text-gray-700 dark:text-gray-400 sm:text-start">
                  Don&apos;t have an account? {""}
                  <Link
                    to="/TailAdmin/signup"
                    className="text-brand-500 hover:text-brand-600 dark:text-brand-400"
                  >
                    Sign Up
                  </Link>
                </p>
              </div>
            </div>
          </div>
        </div>
      </div>
    </AuthLayout>
  );
}
