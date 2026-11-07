// Motiv: shows an available update and opens its release page.

import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Download } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from './ui/dialog';
import { Button } from './ui/button';
import { UpdateInfo } from '@/services/updateService';

interface UpdateDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  updateInfo: UpdateInfo | null;
}

function formatDate(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}

export function UpdateDialog({ open, onOpenChange, updateInfo }: UpdateDialogProps) {
  if (!updateInfo?.available) return null;

  const openDownload = async () => {
    if (updateInfo.downloadUrl) {
      try {
        await invoke('open_external_url', { url: updateInfo.downloadUrl });
      } catch (e) {
        console.error('Failed to open release page:', e);
      }
    }
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[480px]">
        <DialogHeader>
          <DialogTitle>Motiv {updateInfo.version} is available</DialogTitle>
          <DialogDescription>
            You have {updateInfo.currentVersion}
            {updateInfo.date ? `. Released ${formatDate(updateInfo.date)}.` : '.'}
          </DialogDescription>
        </DialogHeader>
        {updateInfo.body && (
          <div className="max-h-60 overflow-y-auto whitespace-pre-wrap rounded-lg bg-gray-50 p-3 text-sm text-gray-700">
            {updateInfo.body}
          </div>
        )}
        <p className="text-xs text-gray-500">The release page has the installers. Download, quit Motiv, and install over this version; your meetings and settings are kept.</p>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>Later</Button>
          <Button onClick={openDownload} className="bg-blue-600 hover:bg-blue-700">
            <Download className="mr-2 h-4 w-4" /> Open download page
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
