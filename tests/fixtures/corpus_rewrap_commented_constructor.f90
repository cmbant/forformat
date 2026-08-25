   INTEGER,  PARAMETER          :: TwNVUnd(3, 9) = RESHAPE( (/ &           ! Undisturbed wind velocity
                                     TwN1VUndx,TwN1VUndy,TwN1VUndz, &
                                     TwN2VUndx,TwN2VUndy,TwN2VUndz, &
                                     TwN3VUndx,TwN3VUndy,TwN3VUndz, &
                                     TwN4VUndx,TwN4VUndy,TwN4VUndz, &
                                     TwN5VUndx,TwN5VUndy,TwN5VUndz, &
                                     TwN6VUndx,TwN6VUndy,TwN6VUndz, &
                                     TwN7VUndx,TwN7VUndy,TwN7VUndz, &
                                     TwN8VUndx,TwN8VUndy,TwN8VUndz, &
                                     TwN9VUndx,TwN9VUndy,TwN9VUndz  &
                                   /), (/3, 9/) )
   INTEGER,  PARAMETER          :: TwNVRel(9) = (/TwN1VRel,TwN2VRel,TwN3VRel,TwN4VRel,TwN5VRel,TwN6VRel,TwN7VRel,TwN8VRel,TwN9VRel/)   ! relative wind speed
