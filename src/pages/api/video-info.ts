import type { APIRoute } from 'astro';
import ffmpeg from 'fluent-ffmpeg';
import ffprobeInstaller from '@ffprobe-installer/ffprobe';
import fs from 'fs/promises';
import path from 'path';

export const prerender = false;

// Set ffprobe path
ffmpeg.setFfprobePath(ffprobeInstaller.path);

interface VideoMetadata {
  duration: number;
  resolution: string;
  codec: string;
  bitrate: number;
  size: number;
  audioCodec: string | null;
  frameRate: string | null;
}

// Format duration in seconds to mm:ss or hh:mm:ss
function formatDuration(seconds: number): string {
  const hrs = Math.floor(seconds / 3600);
  const mins = Math.floor((seconds % 3600) / 60);
  const secs = Math.floor(seconds % 60);

  if (hrs > 0) {
    return `${hrs}:${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
  }
  return `${mins}:${secs.toString().padStart(2, '0')}`;
}

// Format bytes to human readable
function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
}

// Format bitrate to human readable
function formatBitrate(bps: number): string {
  if (bps >= 1000000) {
    return (bps / 1000000).toFixed(1) + ' Mbps';
  }
  return (bps / 1000).toFixed(0) + ' Kbps';
}

// Get friendly codec name
function getCodecName(codec: string): string {
  const codecMap: Record<string, string> = {
    'h264': 'H.264',
    'hevc': 'HEVC (H.265)',
    'h265': 'HEVC (H.265)',
    'vp8': 'VP8',
    'vp9': 'VP9',
    'av1': 'AV1',
    'mpeg4': 'MPEG-4',
    'mjpeg': 'Motion JPEG',
    'prores': 'ProRes',
    'aac': 'AAC',
    'mp3': 'MP3',
    'opus': 'Opus',
    'vorbis': 'Vorbis',
    'pcm_s16le': 'PCM',
    'pcm_s24le': 'PCM 24-bit',
  };
  return codecMap[codec.toLowerCase()] || codec.toUpperCase();
}

export const POST: APIRoute = async ({ request }) => {
  try {
    const { videoUrl } = await request.json();

    if (!videoUrl) {
      return new Response(JSON.stringify({ error: 'Video URL is required' }), {
        status: 400,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    // Convert URL to file path
    const videoPath = videoUrl.replace('/albums/', '');
    const fullPath = path.join(process.cwd(), 'src/content/albums', videoPath);

    // Check if file exists
    try {
      await fs.access(fullPath);
    } catch {
      return new Response(JSON.stringify({ error: 'Video not found' }), {
        status: 404,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    // Extract metadata using ffprobe
    const metadata = await new Promise<any>((resolve, reject) => {
      ffmpeg.ffprobe(fullPath, (err, data) => {
        if (err) reject(err);
        else resolve(data);
      });
    });

    const videoStream = metadata.streams?.find((s: any) => s.codec_type === 'video');
    const audioStream = metadata.streams?.find((s: any) => s.codec_type === 'audio');

    if (!videoStream) {
      return new Response(JSON.stringify({ error: 'No video stream found' }), {
        status: 400,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    // Calculate frame rate from r_frame_rate (e.g., "30000/1001" or "30/1")
    let frameRate: string | null = null;
    if (videoStream.r_frame_rate) {
      const [num, den] = videoStream.r_frame_rate.split('/').map(Number);
      if (den && den !== 0) {
        const fps = num / den;
        frameRate = fps.toFixed(2).replace(/\.?0+$/, '') + ' fps';
      }
    }

    const videoInfo: VideoMetadata = {
      duration: metadata.format?.duration || 0,
      resolution: `${videoStream.width}x${videoStream.height}`,
      codec: videoStream.codec_name || 'Unknown',
      bitrate: metadata.format?.bit_rate || 0,
      size: metadata.format?.size || 0,
      audioCodec: audioStream?.codec_name || null,
      frameRate,
    };

    // Return formatted info for display
    const formattedInfo = {
      duration: formatDuration(videoInfo.duration),
      durationSeconds: videoInfo.duration,
      resolution: videoInfo.resolution,
      width: videoStream.width,
      height: videoStream.height,
      codec: getCodecName(videoInfo.codec),
      codecRaw: videoInfo.codec,
      bitrate: formatBitrate(videoInfo.bitrate),
      bitrateRaw: videoInfo.bitrate,
      size: formatBytes(videoInfo.size),
      sizeBytes: videoInfo.size,
      audioCodec: videoInfo.audioCodec ? getCodecName(videoInfo.audioCodec) : null,
      audioCodecRaw: videoInfo.audioCodec,
      frameRate: videoInfo.frameRate,
    };

    console.log('[VideoInfo] Extracted for:', videoPath, formattedInfo);

    return new Response(JSON.stringify({ info: formattedInfo }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' }
    });
  } catch (error) {
    console.error('[VideoInfo] Error extracting video info:', error);
    return new Response(JSON.stringify({ error: 'Failed to extract video info' }), {
      status: 500,
      headers: { 'Content-Type': 'application/json' }
    });
  }
};
