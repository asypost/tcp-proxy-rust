__kernel void crypt(__private uchar key,__global uchar* data)
{
    data[get_global_id(0)] ^= key;
}