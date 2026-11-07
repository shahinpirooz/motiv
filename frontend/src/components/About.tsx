import React, { useState, useEffect } from "react";
import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
import Image from 'next/image';
import { UpdateDialog } from "./UpdateDialog";
import { updateService, UpdateInfo } from '@/services/updateService';
import { Button } from './ui/button';
import { Loader2, CheckCircle2 } from 'lucide-react';
import { toast } from 'sonner';


export function About() {
    const [currentVersion, setCurrentVersion] = useState<string>('0.5.0');
    const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
    const [isChecking, setIsChecking] = useState(false);
    const [showUpdateDialog, setShowUpdateDialog] = useState(false);

    useEffect(() => {
        // Get current version on mount
        getVersion().then(setCurrentVersion).catch(console.error);
    }, []);

    const openRepo = async () => {
        try {
            await invoke('open_external_url', { url: 'https://github.com/shahinpirooz/motiv' });
        } catch (error) {
            console.error('Failed to open link:', error);
        }
    };

    const handleCheckForUpdates = async () => {
        setIsChecking(true);
        try {
            const info = await updateService.checkForUpdates(true);
            setUpdateInfo(info);
            if (info.available) {
                setShowUpdateDialog(true);
            } else if (info.noReleases) {
                toast.success(`You're on Motiv ${info.currentVersion}`, { description: 'No releases have been published yet.' });
            } else {
                toast.success(`You're up to date (Motiv ${info.currentVersion})`);
            }
        } catch (error: any) {
            console.error('Failed to check for updates:', error);
            const message = typeof error === 'string' ? error : error?.message;
            toast.error('Could not check for updates', { description: message || 'Check your internet connection and try again.' });
        } finally {
            setIsChecking(false);
        }
    };

    return (
        <div className="p-4 space-y-4 h-[80vh] overflow-y-auto">
            {/* Compact Header */}
            <div className="text-center">
                <div className="mb-3">
                    <Image
                        src="icon_128x128.png"
                        alt="Motiv Logo"
                        width={64}
                        height={64}
                        className="mx-auto rounded-2xl"
                    />
                </div>
                <h1 className="text-xl font-bold text-gray-900">Motiv</h1>
                <span className="text-sm text-gray-500"> v{currentVersion}</span>
                <p className="text-medium text-gray-600 mt-1">
                    Meeting notes, summaries and answers, on your own machine.
                </p>
                <div className="mt-3">
                    <Button
                        onClick={handleCheckForUpdates}
                        disabled={isChecking}
                        variant="outline"
                        size="sm"
                        className="text-xs"
                    >
                        {isChecking ? (
                            <>
                                <Loader2 className="h-3 w-3 mr-2 animate-spin" />
                                Checking...
                            </>
                        ) : (
                            <>
                                <CheckCircle2 className="h-3 w-3 mr-2" />
                                Check for Updates
                            </>
                        )}
                    </Button>
                    {updateInfo?.available && (
                        <div className="mt-2 text-xs text-blue-600">
                            Update available: v{updateInfo.version}
                        </div>
                    )}
                </div>
            </div>

            <div className="space-y-3">
                <h2 className="text-base font-semibold text-gray-800">What Motiv does</h2>
                <div className="grid grid-cols-2 gap-2">
                    <div className="bg-gray-50 rounded-lg p-3">
                        <h3 className="font-bold text-sm text-gray-900 mb-1">Private by default</h3>
                        <p className="text-xs text-gray-600 leading-relaxed">Audio and transcription stay on this computer. Only transcript text goes to a cloud model, and only if you choose one.</p>
                    </div>
                    <div className="bg-gray-50 rounded-lg p-3">
                        <h3 className="font-bold text-sm text-gray-900 mb-1">Any model</h3>
                        <p className="text-xs text-gray-600 leading-relaxed">Local models through Ollama or the built-in engine, or Claude, OpenAI, Groq, OpenRouter and compatible endpoints.</p>
                    </div>
                    <div className="bg-gray-50 rounded-lg p-3">
                        <h3 className="font-bold text-sm text-gray-900 mb-1">Ask your meetings</h3>
                        <p className="text-xs text-gray-600 leading-relaxed">Chat with a meeting, search across all of them, and pull out action items and decisions.</p>
                    </div>
                    <div className="bg-gray-50 rounded-lg p-3">
                        <h3 className="font-bold text-sm text-gray-900 mb-1">Knows when you&apos;re in a call</h3>
                        <p className="text-xs text-gray-600 leading-relaxed">Notices Zoom, Teams and Google Meet calls and offers to record them.</p>
                    </div>
                </div>
            </div>

            <div className="pt-2 border-t border-gray-200 text-center space-y-1">
                <button onClick={openRepo} className="text-xs text-blue-600 hover:underline">github.com/shahinpirooz/motiv</button>
                <p className="text-xs text-gray-400">
                    Based on Meetily Community Edition by Zackriya Solutions (MIT License).
                </p>
            </div>

            {/* Update Dialog */}
            <UpdateDialog
                open={showUpdateDialog}
                onOpenChange={setShowUpdateDialog}
                updateInfo={updateInfo}
            />
        </div>

    )
}