// audio_utils_nosndfile.c —— audio_utils.h 的无依赖实现 (板子没有 libsndfile)
// 只支持 16-bit PCM WAV; 重采样用线性插值 (验证够用)
#include "audio_utils.h"

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    char riff[4];
    unsigned int size;
    char wave[4];
    char fmt[4];
    unsigned int fmt_size;
    unsigned short audio_format;
    unsigned short channels;
    unsigned int sample_rate;
    unsigned int byte_rate;
    unsigned short block_align;
    unsigned short bits_per_sample;
} wav_header_t;

static int find_data_chunk(FILE *f, unsigned int *data_size) {
    // 跳过 fmt 块, 找 "data"
    char id[4];
    unsigned int sz;
    for (int guard = 0; guard < 32; guard++) {
        if (fread(id, 1, 4, f) != 4) return -1;
        if (fread(&sz, 4, 1, f) != 1) return -1;
        if (memcmp(id, "data", 4) == 0) {
            *data_size = sz;
            return 0;
        }
        fseek(f, (long)sz + (sz & 1), SEEK_CUR);
    }
    return -1;
}

int read_audio(const char *path, audio_buffer_t *audio) {
    FILE *f = fopen(path, "rb");
    if (!f) return -1;
    wav_header_t h;
    if (fread(&h, sizeof(h), 1, f) != 1) { fclose(f); return -1; }
    if (memcmp(h.riff, "RIFF", 4) || memcmp(h.wave, "WAVE", 4)) { fclose(f); return -1; }
    unsigned int dsize = 0;
    if (find_data_chunk(f, &dsize) != 0) { fclose(f); return -1; }

    int ch = h.channels, sr = (int)h.sample_rate, bps = h.bits_per_sample;
    int nframes = (int)(dsize / (ch * (bps / 8)));
    short *raw = (short *)malloc(dsize);
    if (!raw) { fclose(f); return -1; }
    size_t got = fread(raw, 1, dsize, f);
    fclose(f);
    nframes = (int)(got / (ch * (bps / 8)));

    audio->data = (float *)malloc(sizeof(float) * nframes * ch);
    if (!audio->data) { free(raw); return -1; }
    if (bps == 16) {
        for (int i = 0; i < nframes * ch; i++) audio->data[i] = raw[i] / 32768.0f;
    } else if (bps == 8) {
        unsigned char *u = (unsigned char *)raw;
        for (int i = 0; i < nframes * ch; i++) audio->data[i] = (u[i] - 128) / 128.0f;
    } else {
        free(raw); free(audio->data); audio->data = NULL; return -1;   // 只支持 8/16bit
    }
    free(raw);
    audio->num_frames = nframes;
    audio->num_channels = ch;
    audio->sample_rate = sr;
    return 0;
}

int save_audio(const char *path, float *data, int num_frames, int sample_rate, int num_channels) {
    FILE *f = fopen(path, "wb");
    if (!f) return -1;
    int dsize = num_frames * num_channels * 2;
    wav_header_t h;
    memcpy(h.riff, "RIFF", 4); h.size = dsize + 36; memcpy(h.wave, "WAVE", 4);
    memcpy(h.fmt, "fmt ", 4); h.fmt_size = 16; h.audio_format = 1;
    h.channels = (unsigned short)num_channels;
    h.sample_rate = (unsigned int)sample_rate;
    h.byte_rate = sample_rate * num_channels * 2;
    h.block_align = (unsigned short)(num_channels * 2);
    h.bits_per_sample = 16;
    fwrite(&h, sizeof(h), 1, f);
    fwrite("data", 1, 4, f);
    fwrite(&dsize, 4, 1, f);
    for (int i = 0; i < num_frames * num_channels; i++) {
        float v = data[i];
        if (v > 1.0f) v = 1.0f;
        if (v < -1.0f) v = -1.0f;
        short s = (short)lrintf(v * 32767.0f);
        fwrite(&s, 2, 1, f);
    }
    fclose(f);
    return 0;
}

int convert_channels(audio_buffer_t *audio) {
    if (!audio->data || audio->num_channels != 2) return 0;
    int nf = audio->num_frames;
    for (int i = 0; i < nf; i++)
        audio->data[i] = 0.5f * (audio->data[2 * i] + audio->data[2 * i + 1]);
    audio->num_channels = 1;
    return 0;
}

int resample_audio(audio_buffer_t *audio, int original_sample_rate, int desired_sample_rate) {
    if (!audio->data || original_sample_rate == desired_sample_rate) return 0;
    double ratio = (double)desired_sample_rate / original_sample_rate;
    int ch = audio->num_channels;
    int new_frames = (int)(audio->num_frames * ratio);
    float *out = (float *)malloc(sizeof(float) * new_frames * ch);
    if (!out) return -1;
    for (int i = 0; i < new_frames; i++) {
        double pos = i / ratio;
        int i0 = (int)pos;
        int i1 = i0 + 1 < audio->num_frames ? i0 + 1 : audio->num_frames - 1;
        double frac = pos - i0;
        for (int c = 0; c < ch; c++)
            out[i * ch + c] = (float)((1 - frac) * audio->data[i0 * ch + c] +
                                      frac * audio->data[i1 * ch + c]);
    }
    free(audio->data);
    audio->data = out;
    audio->num_frames = new_frames;
    audio->sample_rate = desired_sample_rate;
    return 0;
}
