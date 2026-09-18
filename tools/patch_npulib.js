// patch_npulib.js —— 把公开版 npulib 扩展成 demo 需要的接口
// (加 tensor_desc_s 描述符层 + get_input_cnt + m_input_data_len/m_input_desc/m_output_desc)
const fs = require('fs');
const H = 'D:/Code/npulib/npulib.h';
const C = 'D:/Code/npulib/npulib.cpp';

// ---------- header ----------
let h = fs.readFileSync(H, 'utf8');

h = h.replace('#define MAX_NETWORK_OUTPUT  32',
  '#define MAX_NETWORK_OUTPUT  32\n#define MAX_NPU_IO          64');

const types = `
typedef enum _vip_fmt_e
{
    kFmtFP32 = 0, kFmtFP16 = 1, kFmtUINT8 = 2, kFmtINT8 = 3, kFmtUINT16 = 4,
    kFmtINT16 = 5, kFmtCHAR = 6, kFmtBFP16 = 7, kFmtINT32 = 8, kFmtUINT32 = 9,
    kFmtINT64 = 10, kFmtUINT64 = 11, kFmtFP64 = 12
} vip_fmt_e;

typedef struct _tensor_desc_s
{
    char       name[256];
    unsigned   num_dims;
    unsigned   sizes[8];
    unsigned   elem_bytes;
    vip_fmt_e  data_format;
} tensor_desc_s;

#define _TENSOR_FMT_BYTES(fmt) ( \\
    ((fmt) == kFmtFP32) ? 4u : ((fmt) == kFmtFP16 || (fmt) == kFmtBFP16) ? 2u : \\
    ((fmt) == kFmtINT16 || (fmt) == kFmtUINT16) ? 2u : \\
    ((fmt) == kFmtINT8 || (fmt) == kFmtUINT8 || (fmt) == kFmtCHAR) ? 1u : \\
    ((fmt) == kFmtINT32 || (fmt) == kFmtUINT32) ? 4u : 8u )

class NpuUint`;
h = h.replace('class NpuUint', types);

h = h.replace('    int get_output_cnt(void);',
`    int get_output_cnt(void);
    int get_input_cnt(void) { return m_input_count; }

    /* --- KWS patch: 暴露 tensor 描述符 (上层 pack/unpack 需要 sizes/format/name) --- */
    tensor_desc_s   m_input_desc[MAX_NPU_IO];
    tensor_desc_s   m_output_desc[MAX_NPU_IO];
    unsigned int    m_input_data_len[MAX_NPU_IO];

    static void fill_desc(tensor_desc_s *d, const char *name, int num_dims,
                          const unsigned *sizes, unsigned data_format)
    {
        memset(d, 0, sizeof(*d));
        if (name) { strncpy(d->name, name, sizeof(d->name) - 1); }
        d->num_dims   = (unsigned)(num_dims > 0 ? num_dims : 0);
        for (int k = 0; k < 8; ++k)
            d->sizes[k] = (k < (int)d->num_dims) ? sizes[k] : 0;
        d->data_format = (vip_fmt_e)data_format;
        d->elem_bytes  = _TENSOR_FMT_BYTES(d->data_format);
    }
    static unsigned desc_elems(const tensor_desc_s *d)
    {
        unsigned n = 1;
        for (unsigned k = 0; k < d->num_dims && k < 8; ++k) n *= d->sizes[k];
        return n;
    }`);
fs.writeFileSync(H, h);
console.log('header patched:', h.includes('tensor_desc_s') && h.includes('get_input_cnt'));

// ---------- cpp ----------
let c = fs.readFileSync(C, 'utf8');

const inFill = `        /* --- KWS patch: 填描述符 (在创建 buffer 前, 用已 query 出的 param) --- */
        fill_desc(&m_input_desc[i], name, param.num_of_dims, param.sizes, param.data_format);
        m_input_data_len[i] = desc_elems(&m_input_desc[i]);
`;
const inAnchor = '        status = vip_create_buffer(&param, sizeof(param), (vip_buffer*)&m_input_buffers[i]);';
if (!c.includes(inAnchor)) { throw new Error('input anchor not found'); }
c = c.replace(inAnchor, inFill + inAnchor);

const outFill = `        /* --- KWS patch: 填输出描述符 --- */
        fill_desc(&m_output_desc[i], name, param.num_of_dims, param.sizes, param.data_format);
`;
const outAnchor = '        status = vip_create_buffer(&param, sizeof(param), (vip_buffer*)&m_output_buffers[i]);';
if (!c.includes(outAnchor)) { throw new Error('output anchor not found'); }
c = c.replace(outAnchor, outFill + outAnchor);

fs.writeFileSync(C, c);
console.log('cpp patched:', c.includes('fill_desc(&m_input_desc[i]'), c.includes('fill_desc(&m_output_desc[i]'));
