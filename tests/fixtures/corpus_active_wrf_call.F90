subroutine p(nlst, did, ncid)
  if (nlst(did)%channel_only == 0 .and. &
      nlst(did)%channelBucket_only == 0) &
    call w_rst_rt_nc2(ncid, RT_DOMAIN(did)%IXRT, RT_DOMAIN(did)%JXRT, &
      RT_DOMAIN(did)%overland%streams_and_lakes%surface_water_to_lake, "lake_inflort")
end subroutine p
