import React, { useState, useEffect } from 'react';
import { Play, Pause, Clock, User, Mail, Briefcase, Calendar, Award } from 'lucide-react';
import { invoke } from "@tauri-apps/api/core";
import { Link } from "react-router-dom";

export default function Dashboard() {
  const [isRunning, setIsRunning] = useState(false);
  const [time, setTime] = useState(0);

  useEffect(() => {
    let interval;
    if (isRunning) {
      interval = setInterval(() => {
        setTime(prevTime => prevTime + 1);
      }, 1000);
    }
    return () => clearInterval(interval);
  }, [isRunning]);

  const handleStart = async () => {
    // await invoke("start_screenshot_service");
    // await invoke("start_activity_logging_service");
    await invoke("start_video_recording", {
      fps: 15, // This was the missing key
      container: 'Mp4',
      segmentDuration: 300,
      audio: true,
      audioSource: 'Both',
    });
    setIsRunning(true);
    setTime(0);
  };

  const handleStop = async () => {
    // await invoke("stop_screenshot_service");
    // await invoke("stop_activity_logging_service");
    await invoke("stop_video_recording");
    setIsRunning(false);
  };

  const formatTime = (seconds) => {
    const hrs = Math.floor(seconds / 3600);
    const mins = Math.floor((seconds % 3600) / 60);
    const secs = seconds % 60;
    return {
      hours: hrs.toString().padStart(2, '0'),
      minutes: mins.toString().padStart(2, '0'),
      seconds: secs.toString().padStart(2, '0')
    };
  };



  const user = {
    name: 'Alex Johnson',
    email: 'alex.johnson@example.com',
    role: 'Product Designer',
    joinDate: 'Jan 2024'
  };

  const timeDisplay = formatTime(time);

  return (
    <div className="min-h-screen bg-gradient-to-br from-gray-100 via-white to-gray-20">
      {/* Header */}
      <header className="bg-[#FFFFFF] border-b border-gray-200 shadow-sm">
        <div className="max-w-7xl mx-auto px-6 py-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="w-11 h-11 rounded-xl flex items-center justify-center" style={{ backgroundColor: '#d42729' }}>
                <Link className="flex justify-center item-center ">
                  <img
                    width={131}
                    height={28}
                    src="images/logo/spectosoft.jpg"
                    alt="Logo"
                  />
                </Link>
              </div>
              <div>
                <h1 className="mb-3 text-2xl font-bold text-black tracking-wide">
                  Specto<span className="text-black/20">Soft</span>
                </h1>
              </div>
            </div>
            <div className="flex items-center gap-3 px-4 py-2 bg-gray-50 rounded-full border border-gray-200">
              <div className="w-9 h-9 rounded-full flex items-center justify-center text-sm font-bold text-white shadow-md" style={{ backgroundColor: '#d42729' }}>
                {user.name.split(' ').map(n => n[0]).join('')}
              </div>
              <div className="hidden sm:block">
                <p className="text-sm font-semibold text-gray-900">{user.name}</p>
                <p className="text-xs text-gray-500">{user.role}</p>
              </div>
            </div>
          </div>
        </div>
      </header>

      {/* Main Content */}
      <main className="max-w-7xl mx-auto px-6 py-10">
        {/* Timer Section */}
        <div className="mb-8">
          <div className="bg-white rounded-3xl shadow-xl border border-gray-200 overflow-hidden">
            <div className="px-8 py-6" style={{ backgroundColor: '#d42729' }}>
              <div className="flex items-center justify-between">
                <h2 className="text-2xl font-bold text-white">Current Session</h2>
                <div className={`px-4 py-1.5 rounded-full text-sm font-semibold ${isRunning ? 'bg-green-400 text-green-900' : 'bg-white/20 text-white'}`}>
                  {isRunning ? '● Recording' : '○ Paused'}
                </div>
              </div>
            </div>

            <div className="p-10">
              {/* Timer Display */}
              <div className="bg-gradient-to-br from-gray-50 to-gray-100 rounded-2xl p-12 mb-8 border border-gray-200">
                <div className="flex items-center justify-center gap-6">
                  <div className="text-center">
                    <div className="text-7xl font-bold text-gray-900 font-mono mb-2">
                      {timeDisplay.hours}
                    </div>
                    <div className="text-sm font-semibold text-gray-500 uppercase tracking-widest">Hours</div>
                  </div>
                  <div className="text-5xl font-bold text-gray-400">:</div>
                  <div className="text-center">
                    <div className="text-7xl font-bold text-gray-900 font-mono mb-2">
                      {timeDisplay.minutes}
                    </div>
                    <div className="text-sm font-semibold text-gray-500 uppercase tracking-widest">Minutes</div>
                  </div>
                  <div className="text-5xl font-bold text-gray-400">:</div>
                  <div className="text-center">
                    <div className="text-7xl font-bold text-gray-900 font-mono mb-2">
                      {timeDisplay.seconds}
                    </div>
                    <div className="text-sm font-semibold text-gray-500 uppercase tracking-widest">Seconds</div>
                  </div>
                </div>
              </div>

              {/* Control Buttons */}
              <div className="flex gap-4">
                <button
                  onClick={handleStart}
                  disabled={isRunning}
                  className={`flex-1 px-8 py-5 rounded-2xl font-bold text-lg transition-all transform hover:scale-105 shadow-lg ${isRunning
                    ? 'bg-gray-200 text-gray-400 cursor-not-allowed shadow-none'
                    : 'bg-gradient-to-r from-[#002768] via-[#0041a8] to-[#005eff] hover:from-[#003080] hover:to-[#006aff] text-white shadow-blue-500/30'
                    }`}
                >
                  <div className="flex items-center justify-center gap-3">
                    <Play className="w-6 h-6" fill="currentColor" />
                    <span>Start</span>
                  </div>
                </button>

                <button
                  onClick={handleStop}
                  disabled={!isRunning && time === 0}
                  className={`flex-1 px-8 py-5 rounded-2xl font-bold text-lg transition-all transform hover:scale-105 shadow-lg ${!isRunning && time === 0
                    ? 'bg-gray-200 text-gray-400 cursor-not-allowed shadow-none'
                    : 'bg-gradient-to-r from-[#d42729] via-[#e52e2e] to-[#ff4b4b] hover:from-[#b81f21] hover:to-[#ff2f2f] text-white shadow-[0_0_15px_rgba(212,39,41,0.5)]'
                    }`}
                >
                  <div className="flex items-center justify-center gap-3">
                    <Pause className="w-6 h-6" fill="currentColor" />
                    <span>Stop</span>
                  </div>
                </button>

              </div>
            </div>
          </div>
        </div>

        {/* Bottom Section */}
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
          {/* User Profile Card */}
          <div className="bg-white rounded-2xl shadow-lg border border-gray-200 p-8">
            <div className="flex items-center gap-3 mb-6">
              <div className="w-10 h-10 bg-indigo-100 rounded-lg flex items-center justify-center">
                <User className="w-5 h-5 text-indigo-600" />
              </div>
              <h2 className="text-xl font-bold text-gray-900">User Profile</h2>
            </div>

            <div className="flex items-center gap-4 mb-6 p-4 rounded-xl border" style={{ backgroundColor: '#d4272920', borderColor: '#d4272940' }}>
              <div className="w-16 h-16 rounded-2xl flex items-center justify-center text-xl font-bold text-white shadow-lg" style={{ backgroundColor: '#d42729' }}>
                {user.name.split(' ').map(n => n[0]).join('')}
              </div>
              <div>
                <p className="text-lg font-bold text-gray-900">{user.name}</p>
                <p className="text-sm text-gray-600">{user.role}</p>
              </div>
            </div>

            <div className="space-y-3">
              <div className="flex items-center gap-3 p-3 bg-gray-50 rounded-xl border border-gray-200">
                <div className="w-9 h-9 bg-blue-100 rounded-lg flex items-center justify-center">
                  <Mail className="w-4 h-4 text-blue-600" />
                </div>
                <div>
                  <p className="text-xs font-semibold text-gray-500 uppercase">Email</p>
                  <p className="text-sm font-medium text-gray-900">{user.email}</p>
                </div>
              </div>

              <div className="flex items-center gap-3 p-3 bg-gray-50 rounded-xl border border-gray-200">
                <div className="w-9 h-9 bg-purple-100 rounded-lg flex items-center justify-center">
                  <Briefcase className="w-4 h-4 text-purple-600" />
                </div>
                <div>
                  <p className="text-xs font-semibold text-gray-500 uppercase">Position</p>
                  <p className="text-sm font-medium text-gray-900">{user.role}</p>
                </div>
              </div>

              <div className="flex items-center gap-3 p-3 bg-gray-50 rounded-xl border border-gray-200">
                <div className="w-9 h-9 bg-green-100 rounded-lg flex items-center justify-center">
                  <Calendar className="w-4 h-4 text-green-600" />
                </div>
                <div>
                  <p className="text-xs font-semibold text-gray-500 uppercase">Joined</p>
                  <p className="text-sm font-medium text-gray-900">{user.joinDate}</p>
                </div>
              </div>
            </div>
          </div>

          {/* Statistics Card */}
          <div className="bg-white rounded-2xl shadow-lg border border-gray-200 p-8">
            <div className="flex items-center gap-3 mb-6">
              <div className="w-10 h-10 bg-amber-100 rounded-lg flex items-center justify-center">
                <Award className="w-5 h-5 text-amber-600" />
              </div>
              <h2 className="text-xl font-bold text-gray-900">Your Statistics</h2>
            </div>

            <div className="grid grid-cols-2 gap-4 mb-6">
              <div className="bg-gradient-to-br from-indigo-50 to-indigo-100 rounded-2xl p-6 border border-indigo-200">
                <div className="text-4xl font-bold text-indigo-600 mb-1">24</div>
                <div className="text-sm font-semibold text-indigo-800 uppercase tracking-wide">Sessions</div>
              </div>
              <div className="bg-gradient-to-br from-purple-50 to-purple-100 rounded-2xl p-6 border border-purple-200">
                <div className="text-4xl font-bold text-purple-600 mb-1">48h</div>
                <div className="text-sm font-semibold text-purple-800 uppercase tracking-wide">Total Time</div>
              </div>
            </div>

            <div className="space-y-3">
              <div className="flex justify-between items-center p-3 bg-gray-50 rounded-lg border border-gray-200">
                <span className="text-sm font-medium text-gray-700">Average Session</span>
                <span className="text-sm font-bold text-gray-900">2h 15m</span>
              </div>
              <div className="flex justify-between items-center p-3 bg-gray-50 rounded-lg border border-gray-200">
                <span className="text-sm font-medium text-gray-700">This Week</span>
                <span className="text-sm font-bold text-gray-900">12h 30m</span>
              </div>
              <div className="flex justify-between items-center p-3 bg-gray-50 rounded-lg border border-gray-200">
                <span className="text-sm font-medium text-gray-700">Productivity Score</span>
                <span className="text-sm font-bold text-green-600">85%</span>
              </div>
            </div>
          </div>
        </div>
      </main>
    </div>
  );
}